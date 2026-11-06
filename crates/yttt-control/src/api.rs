use std::sync::{Arc, LazyLock};

use argon2::{
    Argon2, PasswordHash, PasswordHasher as _, PasswordVerifier as _,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use rand::RngCore as _;
use sqlx::Row as _;
use yttt_relay_ticket::{
    ProofPurpose, Ticket,
    api::{
        AccountCreated, AccountStanding, AccountView, CreateAccount, DeviceEntry, DeviceList,
        DeviceProofBody, Login, RegisterDevice, SessionGranted, SetSuspended, Standing,
        TicketGranted, TicketRequest, UsageReply, UsageReport,
    },
};

use crate::{AppState, Failure, clamp_i64, month_of, now_unix, sha256};

type AppResult<T> = Result<T, Failure>;
type Shared = State<Arc<AppState>>;

const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_BYTES: usize = 256;
const MAX_EMAIL_BYTES: usize = 254;
const MAX_LABEL_CHARS: usize = 64;
/// More accounts than one node report may carry.
const MAX_REPORTED_ACCOUNTS: usize = 100_000;

/// Verified against when the email is unknown, so a login takes the same time
/// whether or not the account exists.
static DECOY_HASH: LazyLock<String> = LazyLock::new(|| hash_password("decoy password"));

/// An account authenticated by its session token.
pub(crate) struct Session {
    account_id: i64,
}

impl FromRequestParts<Arc<AppState>> for Session {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> AppResult<Self> {
        let token = bearer(parts).ok_or(Failure::UNAUTHORIZED)?;
        let account_id: Option<i64> = sqlx::query_scalar(
            "SELECT account_id FROM sessions WHERE token_hash = $1 AND expires_unix > $2",
        )
        .bind(sha256(token.as_bytes()))
        .bind(clamp_i64(now_unix()))
        .fetch_optional(&state.pool)
        .await?;
        account_id
            .map(|account_id| Self { account_id })
            .ok_or(Failure::UNAUTHORIZED)
    }
}

fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

pub(crate) async fn create_account(
    State(state): Shared,
    Json(request): Json<CreateAccount>,
) -> AppResult<(StatusCode, Json<AccountCreated>)> {
    let email = normalize_email(&request.email)?;
    check_password(&request.password)?;
    let password = request.password;
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
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
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn login(
    State(state): Shared,
    Json(request): Json<Login>,
) -> AppResult<Json<SessionGranted>> {
    if request.password.len() > MAX_PASSWORD_BYTES {
        return Err(Failure::UNAUTHORIZED);
    }
    let email = request.email.trim().to_ascii_lowercase();
    let account = sqlx::query("SELECT id, password_hash FROM accounts WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.pool)
        .await?;
    let (account_id, hash) = match &account {
        Some(row) => (
            Some(row.get::<i64, _>("id")),
            row.get::<String, _>("password_hash"),
        ),
        None => (None, DECOY_HASH.clone()),
    };
    let password = request.password;
    let matches = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .expect("verification never panics");
    let Some(account_id) = account_id.filter(|_| matches) else {
        return Err(Failure::UNAUTHORIZED);
    };

    let mut token = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token);
    let token = data_encoding::BASE64URL_NOPAD.encode(&token);
    let expires_unix = now_unix() + state.config.session_ttl().as_secs();
    sqlx::query("DELETE FROM sessions WHERE account_id = $1 AND expires_unix <= $2")
        .bind(account_id)
        .bind(clamp_i64(now_unix()))
        .execute(&state.pool)
        .await?;
    sqlx::query("INSERT INTO sessions (token_hash, account_id, expires_unix) VALUES ($1, $2, $3)")
        .bind(sha256(token.as_bytes()))
        .bind(account_id)
        .bind(clamp_i64(expires_unix))
        .execute(&state.pool)
        .await?;
    Ok(Json(SessionGranted {
        token,
        account_id: account_id as u64,
        expires_unix,
    }))
}

pub(crate) async fn me(State(state): Shared, session: Session) -> AppResult<Json<AccountView>> {
    let row = sqlx::query(
        "SELECT a.email, a.plan_id, a.node_id, n.url, a.suspended, p.monthly_bytes, p.max_devices,
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

    let mut tx = state.pool.begin().await?;
    // Serializes registrations per account so the device limit holds.
    let max_devices: i32 = sqlx::query_scalar(
        "SELECT p.max_devices FROM accounts a JOIN plans p ON p.id = a.plan_id
         WHERE a.id = $1 FOR UPDATE OF a",
    )
    .bind(session.account_id)
    .fetch_one(&mut *tx)
    .await?;
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT account_id FROM devices WHERE endpoint_id = $1")
            .bind(&endpoint_id[..])
            .fetch_optional(&mut *tx)
            .await?;
    let now = clamp_i64(now_unix());
    let status = match owner {
        Some(owner) if owner != session.account_id => {
            return Err(Failure::conflict("the device belongs to another account"));
        }
        Some(_) => {
            sqlx::query("UPDATE devices SET label = $2 WHERE endpoint_id = $1")
                .bind(&endpoint_id[..])
                .bind(label)
                .execute(&mut *tx)
                .await?;
            StatusCode::OK
        }
        None => {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE account_id = $1")
                    .bind(session.account_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if count >= i64::from(max_devices) {
                return Err(Failure::forbidden("the plan's device limit is reached"));
            }
            sqlx::query(
                "INSERT INTO devices (endpoint_id, account_id, label, registered_unix)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(&endpoint_id[..])
            .bind(session.account_id)
            .bind(label)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            StatusCode::CREATED
        }
    };
    let registered: i64 =
        sqlx::query_scalar("SELECT registered_unix FROM devices WHERE endpoint_id = $1")
            .bind(&endpoint_id[..])
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok((
        status,
        Json(DeviceEntry {
            endpoint_id: yttt_relay_ticket::hex(&endpoint_id),
            label: label.to_owned(),
            registered_unix: registered as u64,
        }),
    ))
}

pub(crate) async fn list_devices(
    State(state): Shared,
    session: Session,
) -> AppResult<Json<DeviceList>> {
    let rows = sqlx::query(
        "SELECT endpoint_id, label, registered_unix FROM devices
         WHERE account_id = $1 ORDER BY registered_unix, endpoint_id",
    )
    .bind(session.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(DeviceList {
        devices: rows
            .iter()
            .map(|row| DeviceEntry {
                endpoint_id: yttt_relay_ticket::hex(&row.get::<Vec<u8>, _>("endpoint_id")),
                label: row.get("label"),
                registered_unix: row.get::<i64, _>("registered_unix") as u64,
            })
            .collect(),
    }))
}

pub(crate) async fn remove_device(
    State(state): Shared,
    session: Session,
    Path(endpoint_id): Path<String>,
) -> AppResult<StatusCode> {
    let endpoint_id = yttt_relay_ticket::parse_key(&endpoint_id).ok_or_else(Failure::not_found)?;
    let removed = sqlx::query("DELETE FROM devices WHERE endpoint_id = $1 AND account_id = $2")
        .bind(&endpoint_id[..])
        .bind(session.account_id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(Failure::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn issue_ticket(
    State(state): Shared,
    session: Session,
    Json(request): Json<TicketRequest>,
) -> AppResult<Json<TicketGranted>> {
    let endpoint_id = proven_device(&request.proof, ProofPurpose::IssueTicket, &session)?;
    let row = sqlx::query(
        "SELECT a.suspended, a.node_id, n.url, p.rate_bps, p.burst_bytes, p.monthly_bytes,
                p.throttled_rate_bps, COALESCE(u.upstream_bytes, 0) AS used
         FROM devices d
         JOIN accounts a ON a.id = d.account_id
         JOIN plans p ON p.id = a.plan_id
         JOIN nodes n ON n.id = a.node_id
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $3
         WHERE d.endpoint_id = $1 AND d.account_id = $2",
    )
    .bind(&endpoint_id[..])
    .bind(session.account_id)
    .bind(month_of(now_unix()))
    .fetch_optional(&state.pool)
    .await?
    .ok_or(Failure::forbidden(
        "the device is not registered to this account",
    ))?;
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
    let node_id: String = row.get("node_id");
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
    let token = bearer(&parts).ok_or(Failure::UNAUTHORIZED)?;
    let known: Option<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM nodes WHERE id = $1")
        .bind(&node_id)
        .fetch_optional(&state.pool)
        .await?;
    if known.is_none_or(|hash| hash != sha256(token.as_bytes())) {
        return Err(Failure::UNAUTHORIZED);
    }
    if report.accounts.len() > MAX_REPORTED_ACCOUNTS {
        return Err(Failure::bad_request("too many accounts in one report"));
    }

    let month = month_of(now_unix());
    let mut tx = state.pool.begin().await?;
    for usage in report
        .accounts
        .iter()
        .filter(|usage| usage.upstream_bytes > 0)
    {
        sqlx::query(
            "INSERT INTO usage_months (account_id, month, upstream_bytes)
             SELECT id, $2, $3 FROM accounts WHERE id = $1 AND node_id = $4
             ON CONFLICT (account_id, month)
             DO UPDATE SET upstream_bytes = usage_months.upstream_bytes + EXCLUDED.upstream_bytes",
        )
        .bind(clamp_i64(usage.account_id))
        .bind(month)
        .bind(clamp_i64(usage.upstream_bytes))
        .bind(&node_id)
        .execute(&mut *tx)
        .await?;
    }
    let ids: Vec<i64> = report
        .accounts
        .iter()
        .map(|usage| clamp_i64(usage.account_id))
        .collect();
    let rows = sqlx::query(
        "SELECT a.id, a.suspended, p.rate_bps, p.burst_bytes, p.monthly_bytes,
                p.throttled_rate_bps, COALESCE(u.upstream_bytes, 0) AS used
         FROM accounts a
         JOIN plans p ON p.id = a.plan_id
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $2
         WHERE a.id = ANY($1) AND a.node_id = $3",
    )
    .bind(&ids)
    .bind(month)
    .bind(&node_id)
    .fetch_all(&mut *tx)
    .await?;
    let reported: Vec<Vec<u8>> = report
        .accounts
        .iter()
        .flat_map(|usage| &usage.endpoints)
        .filter_map(|key| yttt_relay_ticket::parse_key(key))
        .map(|key| key.to_vec())
        .collect();
    let registered: Vec<(Vec<u8>, i64)> =
        sqlx::query_as("SELECT endpoint_id, account_id FROM devices WHERE endpoint_id = ANY($1)")
            .bind(&reported)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;

    let standings = report
        .accounts
        .iter()
        .map(|usage| {
            let row = rows
                .iter()
                .find(|row| row.get::<i64, _>("id") as u64 == usage.account_id);
            let standing = match row {
                // Unknown or deleted accounts get nothing.
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
            // A device stays only while it is registered to this account. An
            // account on another node learns nothing from this node's report.
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
                        !registered.iter().any(|(endpoint, account)| {
                            endpoint[..] == key[..] && *account as u64 == usage.account_id
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
    }))
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

fn normalize_email(email: &str) -> AppResult<String> {
    let email = email.trim().to_ascii_lowercase();
    let valid = email.len() <= MAX_EMAIL_BYTES
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    valid
        .then_some(email)
        .ok_or(Failure::bad_request("the email address is invalid"))
}

fn check_password(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD_CHARS || password.len() > MAX_PASSWORD_BYTES {
        return Err(Failure::bad_request("a password is 8 to 256 characters"));
    }
    Ok(())
}

fn hash_password(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .expect("default Argon2 parameters are valid")
        .to_string()
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}
