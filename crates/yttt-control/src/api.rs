use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, request::Parts},
};
use sqlx::Row as _;
use yttt_relay_ticket::{
    ProofPurpose, Ticket,
    api::{
        AccountCreated, AccountStanding, AccountView, CreateAccount, DeviceEntry, DeviceList,
        DeviceProofBody, DeviceRole, Login, RegisterDevice, Revocation, SessionGranted,
        SetSuspended, Standing, TicketGranted, TicketRequest, UsageReply, UsageReport,
    },
};

use crate::{
    AppState, Failure,
    auth::{self, AnySession, Attempt, Session},
    clamp_i64, day_of, month_of, now_unix, placement, sha256,
    web::{check_password, normalize_email},
};

type AppResult<T> = Result<T, Failure>;
type Shared = State<Arc<AppState>>;

const MAX_LABEL_CHARS: usize = 64;
/// More accounts than one node report may carry.
const MAX_REPORTED_ACCOUNTS: usize = 100_000;
/// Revocations per reply; a longer backlog arrives over the next reports.
const MAX_REVOCATIONS_PER_REPLY: i64 = 5_000;

pub(crate) async fn create_account(
    State(state): Shared,
    Json(request): Json<CreateAccount>,
) -> AppResult<(StatusCode, Json<AccountCreated>)> {
    let email = normalize_email(&request.email)?;
    check_password(&request.password)?;
    let password = request.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
        .await
        .expect("hashing never panics");
    let created: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO accounts (email, password_hash, plan_id, node_id, created_unix)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(&email)
    .bind(hash)
    .bind(&request.plan)
    .bind(&request.node)
    .bind(clamp_i64(now_unix()))
    .fetch_one(&state.pool)
    .await;
    match created {
        Ok(id) => Ok((
            StatusCode::CREATED,
            Json(AccountCreated {
                account_id: id as u64,
            }),
        )),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Err(Failure::conflict("the email is taken"))
        }
        Err(sqlx::Error::Database(error)) if error.is_foreign_key_violation() => {
            Err(Failure::bad_request("unknown plan or node"))
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn set_suspended(
    State(state): Shared,
    Path(account_id): Path<i64>,
    Json(request): Json<SetSuspended>,
) -> AppResult<StatusCode> {
    let updated = sqlx::query("UPDATE accounts SET suspended = $2 WHERE id = $1")
        .bind(account_id)
        .bind(request.suspended)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(Failure::not_found());
    }
    if request.suspended {
        auth::end_sessions(&state, account_id, None).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn login(
    State(state): Shared,
    _attempt: Attempt,
    Json(request): Json<Login>,
) -> AppResult<Json<SessionGranted>> {
    let email = request.email.trim().to_ascii_lowercase();
    let account_id = auth::check_login(&state, &email, &request.password).await?;
    let (token, expires_unix) = auth::open_session(&state, account_id, auth::Scope::Device).await?;
    Ok(Json(SessionGranted {
        token,
        account_id: account_id as u64,
        expires_unix,
    }))
}

pub(crate) async fn me(State(state): Shared, session: AnySession) -> AppResult<Json<AccountView>> {
    let row = sqlx::query(
        "SELECT a.email, a.plan_id, a.node_id, n.url, a.suspended, p.monthly_bytes, p.max_devices,
                p.name AS plan_name, p.description AS plan_description, p.rate_bps,
                p.throttled_rate_bps, n.name AS node_name,
                COALESCE(u.upstream_bytes, 0) AS used,
                (SELECT COUNT(*) FROM devices d WHERE d.account_id = a.id) AS devices
         FROM accounts a
         JOIN plans p ON p.id = a.plan_id
         JOIN nodes n ON n.id = a.node_id
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $2
         WHERE a.id = $1",
    )
    .bind(session.account_id)
    .bind(month_of(now_unix()))
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(AccountView {
        account_id: session.account_id as u64,
        email: row.get("email"),
        plan: row.get("plan_id"),
        node_id: row.get("node_id"),
        relay_url: row.get("url"),
        month_upstream_bytes: row.get::<i64, _>("used") as u64,
        monthly_bytes: row.get::<i64, _>("monthly_bytes") as u64,
        suspended: row.get("suspended"),
        devices: row.get::<i64, _>("devices") as u32,
        max_devices: row.get::<i32, _>("max_devices") as u32,
        role: session.role,
        plan_name: row.get("plan_name"),
        plan_description: row.get("plan_description"),
        node_name: row.get("node_name"),
        rate_bps: row.get::<i32, _>("rate_bps") as u32,
        throttled_rate_bps: row.get::<i32, _>("throttled_rate_bps") as u32,
    }))
}

pub(crate) async fn register_device(
    State(state): Shared,
    session: Session,
    Json(request): Json<RegisterDevice>,
) -> AppResult<(StatusCode, Json<DeviceEntry>)> {
    let label = request.label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(Failure::bad_request("a device label is 1 to 64 characters"));
    }
    let endpoint_id = proven_device(&request.proof, ProofPurpose::RegisterDevice, &session)?;
    let follows_host = request
        .follows_host
        .as_deref()
        .map(|host| {
            yttt_relay_ticket::parse_key(host)
                .ok_or(Failure::bad_request("the Host key must be 64 hex digits"))
        })
        .transpose()?;
    let mut tx = state.pool.begin().await?;
    let status = placement::register(
        &mut tx,
        session.account_id,
        &endpoint_id,
        label,
        request.role.unwrap_or(DeviceRole::Unknown),
        follows_host,
    )
    .await?;
    let row = sqlx::query(&format!(
        "SELECT {} FROM devices WHERE endpoint_id = $1",
        placement::DEVICE_COLUMNS
    ))
    .bind(&endpoint_id[..])
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((status, Json(placement::entry(&row))))
}

pub(crate) async fn list_devices(
    State(state): Shared,
    session: Session,
) -> AppResult<Json<DeviceList>> {
    let rows = sqlx::query(&format!(
        "SELECT {} FROM devices WHERE account_id = $1 ORDER BY registered_unix, endpoint_id",
        placement::DEVICE_COLUMNS
    ))
    .bind(session.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(DeviceList {
        devices: rows.iter().map(placement::entry).collect(),
    }))
}

pub(crate) async fn remove_device(
    State(state): Shared,
    session: Session,
    Path(endpoint_id): Path<String>,
) -> AppResult<StatusCode> {
    let endpoint_id = yttt_relay_ticket::parse_key(&endpoint_id).ok_or_else(Failure::not_found)?;
    let mut tx = state.pool.begin().await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM devices WHERE endpoint_id = $1 AND account_id = $2)",
    )
    .bind(&endpoint_id[..])
    .bind(session.account_id)
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        return Err(Failure::not_found());
    }
    // A Host takes its Clients along.
    placement::revoke_removed(&mut tx, &endpoint_id).await?;
    sqlx::query("DELETE FROM devices WHERE endpoint_id = $1")
        .bind(&endpoint_id[..])
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn issue_ticket(
    State(state): Shared,
    session: Session,
    Json(request): Json<TicketRequest>,
) -> AppResult<Json<TicketGranted>> {
    let endpoint_id = proven_device(&request.proof, ProofPurpose::IssueTicket, &session)?;
    let host = request
        .host
        .as_deref()
        .map(|host| {
            yttt_relay_ticket::parse_key(host)
                .ok_or(Failure::bad_request("the Host key must be 64 hex digits"))
        })
        .transpose()?;
    let mut tx = state.pool.begin().await?;
    let node_id = placement::ticket_node(&mut tx, session.account_id, &endpoint_id, host).await?;
    let row = sqlx::query(
        "SELECT a.suspended, n.url, p.rate_bps, p.burst_bytes, p.monthly_bytes,
                p.throttled_rate_bps, COALESCE(u.upstream_bytes, 0) AS used
         FROM accounts a
         JOIN plans p ON p.id = a.plan_id
         JOIN nodes n ON n.id = $3
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $2
         WHERE a.id = $1",
    )
    .bind(session.account_id)
    .bind(month_of(now_unix()))
    .bind(&node_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if row.get::<bool, _>("suspended") {
        return Err(Failure::forbidden("the account is suspended"));
    }
    let over_quota = row.get::<i64, _>("used") >= row.get::<i64, _>("monthly_bytes");
    let (rate_bps, burst_bytes) = if over_quota {
        let throttled = row.get::<i32, _>("throttled_rate_bps") as u32;
        (throttled, throttled)
    } else {
        (
            row.get::<i32, _>("rate_bps") as u32,
            row.get::<i32, _>("burst_bytes") as u32,
        )
    };
    let issued_unix = now_unix();
    let expires_unix = issued_unix + state.config.ticket_ttl().as_secs();
    let token = state.signer.issue(&Ticket {
        account_id: session.account_id as u64,
        endpoint_id,
        node_id: node_id.clone(),
        rate_bps,
        burst_bytes,
        issued_unix,
        expires_unix,
    });
    Ok(Json(TicketGranted {
        token,
        node_id,
        relay_url: row.get("url"),
        expires_unix,
    }))
}

pub(crate) async fn node_usage(
    State(state): Shared,
    Path(node_id): Path<String>,
    parts: Parts,
    Json(report): Json<UsageReport>,
) -> AppResult<Json<UsageReply>> {
    let token = auth::bearer(&parts.headers).ok_or(Failure::UNAUTHORIZED)?;
    let node = sqlx::query("SELECT token_hash, state FROM nodes WHERE id = $1")
        .bind(&node_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(Failure::UNAUTHORIZED)?;
    // A node that has not enrolled has no token, and nothing matches it.
    let known: Option<Vec<u8>> = node.get("token_hash");
    if known.is_none_or(|hash| hash != sha256(token.as_bytes())) {
        return Err(Failure::UNAUTHORIZED);
    }
    // A disabled node is still answered, so its connected devices are told to
    // leave rather than relayed unbilled until their tickets expire.
    let disabled = node.get::<String, _>("state") == "disabled";
    let connected: usize = report
        .accounts
        .iter()
        .map(|usage| usage.endpoints.len())
        .sum();
    sqlx::query(
        "UPDATE nodes SET last_report_unix = $2, last_connected_devices = $3,
             version = COALESCE($4, version)
         WHERE id = $1",
    )
    .bind(&node_id)
    .bind(clamp_i64(now_unix()))
    .bind(i32::try_from(connected).unwrap_or(i32::MAX))
    .bind(
        report
            .version
            .as_deref()
            .map(|version| version.chars().take(64).collect::<String>()),
    )
    .execute(&state.pool)
    .await?;
    if report.accounts.len() > MAX_REPORTED_ACCOUNTS {
        return Err(Failure::bad_request("too many accounts in one report"));
    }

    let now = now_unix();
    let month = month_of(now);
    let day = day_of(now);
    record_load(
        &state,
        &node_id,
        now,
        connected,
        report
            .accounts
            .iter()
            .map(|usage| usage.upstream_bytes)
            .sum(),
    )
    .await?;
    let reported: Vec<Vec<u8>> = report
        .accounts
        .iter()
        .flat_map(|usage| &usage.endpoints)
        .filter_map(|key| yttt_relay_ticket::parse_key(key))
        .map(|key| key.to_vec())
        .collect();
    let ids: Vec<i64> = report
        .accounts
        .iter()
        .map(|usage| clamp_i64(usage.account_id))
        .collect();
    let mut tx = state.pool.begin().await?;
    let registered: Vec<(Vec<u8>, i64, String)> = sqlx::query_as(
        "SELECT endpoint_id, account_id, node_id FROM devices WHERE endpoint_id = ANY($1)",
    )
    .bind(&reported)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE devices SET last_seen_unix = $3 WHERE endpoint_id = ANY($1) AND node_id = $2",
    )
    .bind(&reported)
    .bind(&node_id)
    .bind(clamp_i64(now))
    .execute(&mut *tx)
    .await?;
    let on_node: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM accounts WHERE node_id = $1 AND id = ANY($2)
         UNION
         SELECT account_id FROM devices WHERE node_id = $1 AND account_id = ANY($2)",
    )
    .bind(&node_id)
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    // A node bills and steers only accounts it has to do with: it is their
    // default node or one of their devices belongs on it. Reporting someone
    // else's device keys, which are public, earns nothing. A device that just
    // moved away relays unbilled here until its revocation lands.
    let related = |account: u64| on_node.contains(&clamp_i64(account));
    for usage in report
        .accounts
        .iter()
        .filter(|usage| usage.upstream_bytes > 0 && related(usage.account_id))
    {
        sqlx::query(
            "INSERT INTO usage_months (account_id, month, upstream_bytes)
             SELECT id, $2, $3 FROM accounts WHERE id = $1
             ON CONFLICT (account_id, month)
             DO UPDATE SET upstream_bytes = usage_months.upstream_bytes + EXCLUDED.upstream_bytes",
        )
        .bind(clamp_i64(usage.account_id))
        .bind(month)
        .bind(clamp_i64(usage.upstream_bytes))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO usage_days (account_id, day, node_id, upstream_bytes)
             SELECT id, $2, $4, $3 FROM accounts WHERE id = $1
             ON CONFLICT (account_id, day, node_id)
             DO UPDATE SET upstream_bytes = usage_days.upstream_bytes + EXCLUDED.upstream_bytes",
        )
        .bind(clamp_i64(usage.account_id))
        .bind(day)
        .bind(clamp_i64(usage.upstream_bytes))
        .bind(&node_id)
        .execute(&mut *tx)
        .await?;
    }
    let rows = sqlx::query(
        "SELECT a.id, a.suspended, p.rate_bps, p.burst_bytes, p.monthly_bytes,
                p.throttled_rate_bps, COALESCE(u.upstream_bytes, 0) AS used
         FROM accounts a
         JOIN plans p ON p.id = a.plan_id
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $2
         WHERE a.id = ANY($1)",
    )
    .bind(&ids)
    .bind(month)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM node_revocations WHERE revoked_unix < $1")
        .bind(clamp_i64(
            now.saturating_sub(placement::REVOCATION_KEEP_SECS),
        ))
        .execute(&mut *tx)
        .await?;
    let revocations: Vec<(i64, Vec<u8>, i64)> = sqlx::query_as(
        "SELECT id, endpoint_id, revoked_unix FROM node_revocations
         WHERE node_id = $1 AND id > $2 ORDER BY id LIMIT $3",
    )
    .bind(&node_id)
    .bind(clamp_i64(report.revocations_after.unwrap_or(0)))
    .bind(MAX_REVOCATIONS_PER_REPLY)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let standings = report
        .accounts
        .iter()
        .map(|usage| {
            let row = rows
                .iter()
                .find(|row| row.get::<i64, _>("id") as u64 == usage.account_id)
                .filter(|_| related(usage.account_id));
            let standing = match row {
                // Unknown or deleted accounts, accounts this node has nothing
                // to do with, and everyone on a disabled node get nothing.
                _ if disabled => Standing::Suspended,
                None => Standing::Suspended,
                Some(row) if row.get::<bool, _>("suspended") => Standing::Suspended,
                Some(row) if row.get::<i64, _>("used") >= row.get::<i64, _>("monthly_bytes") => {
                    let rate = row.get::<i32, _>("throttled_rate_bps") as u32;
                    Standing::Throttled {
                        rate_bps: rate,
                        burst_bytes: rate,
                    }
                }
                Some(row) => Standing::Active {
                    rate_bps: row.get::<i32, _>("rate_bps") as u32,
                    burst_bytes: row.get::<i32, _>("burst_bytes") as u32,
                },
            };
            // A device stays only while it is registered to this account and
            // belongs on this node. An unrelated account learns nothing.
            let revoked_endpoints = if row.is_none() {
                Vec::new()
            } else {
                usage
                    .endpoints
                    .iter()
                    .filter(|key| {
                        let Some(key) = yttt_relay_ticket::parse_key(key) else {
                            return false;
                        };
                        !registered.iter().any(|(endpoint, account, node)| {
                            endpoint[..] == key[..]
                                && *account as u64 == usage.account_id
                                && *node == node_id
                        })
                    })
                    .cloned()
                    .collect()
            };
            AccountStanding {
                account_id: usage.account_id,
                standing,
                revoked_endpoints,
            }
        })
        .collect();
    Ok(Json(UsageReply {
        accounts: standings,
        revocations: revocations
            .into_iter()
            .map(|(id, endpoint, at)| Revocation {
                id: id as u64,
                endpoint_id: yttt_relay_ticket::hex(&endpoint),
                revoked_unix: at.max(0) as u64,
            })
            .collect(),
    }))
}

/// Five-minute samples kept for the console's day view.
const SAMPLE_SECS: u64 = 5 * 60;
const SAMPLES_KEEP_SECS: u64 = 48 * 3600;
const NODE_DAYS_KEEP: i32 = 90;

/// Adds one report to the node's load history.
async fn record_load(
    state: &AppState,
    node_id: &str,
    now: u64,
    connected: usize,
    upstream_bytes: u64,
) -> AppResult<()> {
    let connected = i32::try_from(connected).unwrap_or(i32::MAX);
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO node_samples (node_id, bucket_unix, connections_max, upstream_bytes)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (node_id, bucket_unix) DO UPDATE SET
             connections_max = GREATEST(node_samples.connections_max, EXCLUDED.connections_max),
             upstream_bytes = node_samples.upstream_bytes + EXCLUDED.upstream_bytes",
    )
    .bind(node_id)
    .bind(clamp_i64(now - now % SAMPLE_SECS))
    .bind(connected)
    .bind(clamp_i64(upstream_bytes))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO node_days (node_id, day, connections_max) VALUES ($1, $2, $3)
         ON CONFLICT (node_id, day) DO UPDATE SET
             connections_max = GREATEST(node_days.connections_max, EXCLUDED.connections_max)",
    )
    .bind(node_id)
    .bind(day_of(now))
    .bind(connected)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM node_samples WHERE node_id = $1 AND bucket_unix < $2")
        .bind(node_id)
        .bind(clamp_i64(now.saturating_sub(SAMPLES_KEEP_SECS)))
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM node_days WHERE node_id = $1 AND day < $2")
        .bind(node_id)
        .bind(day_of(now) - NODE_DAYS_KEEP)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Checks that the caller holds the device key the proof names, for this
/// request and this account.
fn proven_device(
    proof: &DeviceProofBody,
    purpose: ProofPurpose,
    session: &Session,
) -> AppResult<[u8; 32]> {
    let proof = proof
        .decode()
        .ok_or(Failure::bad_request("the device proof is malformed"))?;
    proof
        .verify(purpose, session.account_id as u64, now_unix())
        .map_err(|_| Failure::forbidden("the device proof was rejected"))?;
    Ok(proof.endpoint_id)
}
