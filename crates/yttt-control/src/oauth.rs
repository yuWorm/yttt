//! Signing in the app and the CLI without a password: the app through the
//! browser (authorization code with PKCE and a loopback redirect, RFC 8252),
//! headless devices with a short code someone confirms on the console
//! (RFC 8628). Both end in a `device` session, which cannot administer or
//! change how the account signs in, so a person tricked into approving
//! someone else's sign-in gives away relay use and nothing more.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use sqlx::Row as _;
use yttt_relay_ticket::api::{
    DeviceAuthorization, DeviceAuthorizationRequest, SessionGranted, TokenRequest, token_error,
};

use crate::{
    AppState, Failure,
    auth::{self, Attempt, ConsoleSession, Scope},
    clamp_i64, now_unix, sha256,
};

type Shared = State<Arc<AppState>>;

const CODE_TTL_SECS: u64 = 60;
const DEVICE_TTL_SECS: u64 = 10 * 60;
const POLL_INTERVAL_SECS: u64 = 5;
const MAX_LABEL_CHARS: usize = 64;
/// Letters and digits nobody misreads: no 0, O, 1 or I.
const USER_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const USER_CODE_LEN: usize = 8;

/// What the console's authorize page asks the control plane about.
#[derive(Deserialize)]
pub(crate) struct AuthorizeRequest {
    redirect_uri: String,
    state: String,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
    #[serde(default)]
    label: String,
    /// Absent to only check the request before showing it.
    #[serde(default)]
    allow: Option<bool>,
}

#[derive(Serialize)]
pub(crate) struct AuthorizeAnswer {
    /// Where the browser goes next; absent when only checking.
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect: Option<String>,
    label: String,
}

/// `POST /v1/oauth/authorize`: checks an app's sign-in request, or answers it
/// for the signed-in account.
pub(crate) async fn authorize(
    State(state): Shared,
    session: ConsoleSession,
    Json(request): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeAnswer>, Failure> {
    // An invalid redirect is never followed: the page shows the error.
    let mut redirect = loopback_redirect(&request.redirect_uri)?;
    if request.code_challenge_method != "S256" || !is_challenge(&request.code_challenge) {
        return Err(Failure::bad_request("the app must use PKCE with S256"));
    }
    if request.state.is_empty() || request.state.len() > 512 {
        return Err(Failure::bad_request("the request has no valid state"));
    }
    let label = label(&request.label);
    let Some(allow) = request.allow else {
        return Ok(Json(AuthorizeAnswer {
            redirect: None,
            label,
        }));
    };
    if !allow {
        redirect
            .query_pairs_mut()
            .append_pair("error", token_error::DENIED)
            .append_pair("state", &request.state);
        return Ok(Json(AuthorizeAnswer {
            redirect: Some(redirect.into()),
            label,
        }));
    }
    if session.suspended {
        return Err(Failure::forbidden("the account is suspended"));
    }
    let code = auth::random_token();
    let now = now_unix();
    sqlx::query("DELETE FROM oauth_codes WHERE expires_unix <= $1")
        .bind(clamp_i64(now))
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "INSERT INTO oauth_codes (code_hash, account_id, challenge, redirect_uri, label, expires_unix)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(sha256(code.as_bytes()))
    .bind(session.account_id)
    .bind(&request.code_challenge)
    .bind(&request.redirect_uri)
    .bind(&label)
    .bind(clamp_i64(now + CODE_TTL_SECS))
    .execute(&state.pool)
    .await?;
    redirect
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &request.state);
    Ok(Json(AuthorizeAnswer {
        redirect: Some(redirect.into()),
        label,
    }))
}

/// `POST /v1/oauth/device`: a headless device asks for a short code.
pub(crate) async fn start_device(
    State(state): Shared,
    _attempt: Attempt,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<DeviceAuthorizationRequest>,
) -> Result<Json<DeviceAuthorization>, Failure> {
    let requester = auth::client_ip(&headers, Some(peer), state.config.trusted_proxy);
    let now = now_unix();
    sqlx::query("DELETE FROM device_grants WHERE expires_unix <= $1")
        .bind(clamp_i64(now))
        .execute(&state.pool)
        .await?;
    let device_code = auth::random_token();
    // A collision with a live code is rare; try another.
    for _ in 0..4 {
        let user_code = user_code();
        let inserted = sqlx::query(
            "INSERT INTO device_grants (device_code_hash, user_code, label, requester_ip,
                                        created_unix, expires_unix)
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (user_code) DO NOTHING",
        )
        .bind(sha256(device_code.as_bytes()))
        .bind(&user_code)
        .bind(label(&request.label))
        .bind(requester.to_string())
        .bind(clamp_i64(now))
        .bind(clamp_i64(now + DEVICE_TTL_SECS))
        .execute(&state.pool)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(Json(DeviceAuthorization {
                device_code,
                user_code: format!("{}-{}", &user_code[..4], &user_code[4..]),
                verification_uri: state
                    .config
                    .public_url
                    .as_deref()
                    .map(|url| format!("{}/device", url.trim_end_matches('/'))),
                interval: POLL_INTERVAL_SECS,
                expires_in: DEVICE_TTL_SECS,
            }));
        }
    }
    Err(Failure::unavailable("try again"))
}

#[derive(Deserialize)]
pub(crate) struct DeviceDecision {
    user_code: String,
    /// Absent to look the code up before deciding.
    #[serde(default)]
    allow: Option<bool>,
}

/// What the console shows before someone confirms a code.
#[derive(Serialize)]
pub(crate) struct DeviceRequestView {
    label: String,
    requester_ip: String,
    created_unix: u64,
}

/// `POST /v1/oauth/device/confirm`: looks up, allows or denies a short code
/// for the signed-in account. Every call spends a guess.
pub(crate) async fn confirm_device(
    State(state): Shared,
    _attempt: Attempt,
    session: ConsoleSession,
    Json(request): Json<DeviceDecision>,
) -> Result<Json<DeviceRequestView>, Failure> {
    let user_code = normalize_user_code(&request.user_code);
    let now = clamp_i64(now_unix());
    let row = sqlx::query(
        "SELECT label, requester_ip, created_unix FROM device_grants
         WHERE user_code = $1 AND expires_unix > $2 AND account_id IS NULL AND NOT denied",
    )
    .bind(&user_code)
    .bind(now)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(Failure::bad_request("the code is wrong or has expired"))?;
    let view = DeviceRequestView {
        label: row.get("label"),
        requester_ip: row.get("requester_ip"),
        created_unix: row.get::<i64, _>("created_unix").max(0) as u64,
    };
    match request.allow {
        None => {}
        Some(true) => {
            if session.suspended {
                return Err(Failure::forbidden("the account is suspended"));
            }
            sqlx::query(
                "UPDATE device_grants SET account_id = $2
                 WHERE user_code = $1 AND account_id IS NULL AND NOT denied",
            )
            .bind(&user_code)
            .bind(session.account_id)
            .execute(&state.pool)
            .await?;
        }
        Some(false) => {
            sqlx::query(
                "UPDATE device_grants SET denied = TRUE
                 WHERE user_code = $1 AND account_id IS NULL",
            )
            .bind(&user_code)
            .execute(&state.pool)
            .await?;
        }
    }
    Ok(Json(view))
}

/// `POST /v1/oauth/token`: an approved sign-in becomes a `device` session.
pub(crate) async fn token(
    State(state): Shared,
    Json(request): Json<TokenRequest>,
) -> Result<Json<SessionGranted>, Failure> {
    let account_id = match request {
        TokenRequest::AuthorizationCode {
            code,
            code_verifier,
            redirect_uri,
        } => redeem_code(&state, &code, &code_verifier, &redirect_uri).await?,
        TokenRequest::DeviceCode { device_code } => redeem_device(&state, &device_code).await?,
    };
    let suspended: crate::db::Flag =
        sqlx::query_scalar("SELECT suspended FROM accounts WHERE id = $1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await?;
    if suspended.0 {
        return Err(Failure::forbidden("the account is suspended"));
    }
    let (token, expires_unix) = auth::open_session(&state, account_id, Scope::Device).await?;
    Ok(Json(SessionGranted {
        token,
        account_id: account_id as u64,
        expires_unix,
    }))
}

async fn redeem_code(
    state: &AppState,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<i64, Failure> {
    // Taken out in one statement: a code works once, even when raced.
    let row = sqlx::query(
        "DELETE FROM oauth_codes WHERE code_hash = $1
         RETURNING account_id, challenge, redirect_uri, expires_unix",
    )
    .bind(sha256(code.as_bytes()))
    .fetch_optional(&state.pool)
    .await?
    .ok_or(Failure::bad_request(token_error::INVALID))?;
    let fresh = row.get::<i64, _>("expires_unix") > clamp_i64(now_unix());
    let challenge: String = row.get("challenge");
    let matches = fresh
        && row.get::<String, _>("redirect_uri") == redirect_uri
        && challenge_of(verifier).is_some_and(|computed| computed == challenge);
    if !matches {
        return Err(Failure::bad_request(token_error::INVALID));
    }
    Ok(row.get("account_id"))
}

async fn redeem_device(state: &AppState, device_code: &str) -> Result<i64, Failure> {
    let hash = sha256(device_code.as_bytes());
    let now = now_unix();
    // Records this poll and reads the one before it, together.
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let row = sqlx::query(&format!(
        "SELECT account_id, denied, expires_unix, last_poll_unix AS previous
         FROM device_grants WHERE device_code_hash = $1{}",
        state.dialect.for_update()
    ))
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Failure::bad_request(token_error::INVALID))?;
    sqlx::query("UPDATE device_grants SET last_poll_unix = $2 WHERE device_code_hash = $1")
        .bind(&hash)
        .bind(clamp_i64(now))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if row.get::<i64, _>("expires_unix") <= clamp_i64(now) {
        return Err(Failure::bad_request(token_error::EXPIRED));
    }
    if crate::db::flag(&row, "denied") {
        delete_grant(state, &hash).await?;
        return Err(Failure::bad_request(token_error::DENIED));
    }
    if let Some(account_id) = row.get::<Option<i64>, _>("account_id") {
        // Only the poll that deletes the grant gets the session.
        if delete_grant(state, &hash).await? {
            return Ok(account_id);
        }
        return Err(Failure::bad_request(token_error::INVALID));
    }
    // A second of slack for timers that fire a little early.
    let previous = row.get::<i64, _>("previous").max(0) as u64;
    if now.saturating_sub(previous) + 1 < POLL_INTERVAL_SECS {
        return Err(Failure::bad_request(token_error::SLOW_DOWN));
    }
    Err(Failure::bad_request(token_error::PENDING))
}

async fn delete_grant(state: &AppState, hash: &[u8]) -> Result<bool, Failure> {
    Ok(
        sqlx::query("DELETE FROM device_grants WHERE device_code_hash = $1")
            .bind(hash)
            .execute(&state.pool)
            .await?
            .rows_affected()
            == 1,
    )
}

/// The app's redirect, which must come back to the machine that signed in:
/// plain HTTP to 127.0.0.1 or [::1] on any port (RFC 8252 §7.3), nothing
/// else in the authority and no fragment.
fn loopback_redirect(uri: &str) -> Result<url::Url, Failure> {
    let invalid = || Failure::bad_request("the app's redirect is not a loopback address");
    let url = url::Url::parse(uri).map_err(|_| invalid())?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Some(url::Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        _ => false,
    };
    let valid = url.scheme() == "http"
        && loopback
        && url.port().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none();
    valid.then_some(url).ok_or_else(invalid)
}

fn is_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// The S256 challenge of a PKCE verifier (RFC 7636 §4.1–4.2).
fn challenge_of(verifier: &str) -> Option<String> {
    let valid = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte));
    valid.then(|| data_encoding::BASE64URL_NOPAD.encode(&sha256(verifier.as_bytes())))
}

fn label(label: &str) -> String {
    let label: String = label
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    if label.is_empty() {
        "device".into()
    } else {
        label
    }
}

fn user_code() -> String {
    use rand::Rng as _;
    let mut rng = rand::rngs::OsRng;
    (0..USER_CODE_LEN)
        .map(|_| char::from(USER_CODE_ALPHABET[rng.gen_range(0..USER_CODE_ALPHABET.len())]))
        .collect()
}

fn normalize_user_code(code: &str) -> String {
    code.chars()
        .filter(|character| !matches!(character, '-' | ' '))
        .map(|character| character.to_ascii_uppercase())
        .take(USER_CODE_LEN + 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_redirects_are_followed() {
        for good in [
            "http://127.0.0.1:53682/callback",
            "http://[::1]:4000/callback?x=1",
        ] {
            assert!(loopback_redirect(good).is_ok(), "{good}");
        }
        for bad in [
            "https://127.0.0.1:53682/callback",
            "http://localhost:53682/callback",
            "http://127.0.0.1/callback",
            "http://127.0.0.1:1@evil.example/callback",
            "http://user@127.0.0.1:80/callback",
            "http://127.0.0.1:80/callback#frag",
            "http://127.0.0.2:80/callback",
            "http://evil.example:80/callback",
            "javascript:alert(1)",
            "",
        ] {
            assert!(loopback_redirect(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pkce_matches_the_rfc_example() {
        // RFC 7636 Appendix B.
        assert_eq!(
            challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").as_deref(),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        assert!(challenge_of("short").is_none());
        assert!(is_challenge("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(!is_challenge("plain"));
    }

    #[test]
    fn user_codes_are_easy_to_read_and_type() {
        let code = user_code();
        assert_eq!(code.len(), USER_CODE_LEN);
        assert!(code.bytes().all(|byte| USER_CODE_ALPHABET.contains(&byte)));
        assert_eq!(normalize_user_code("abcd-efgh"), "ABCDEFGH");
        assert_eq!(normalize_user_code(" ABCD EFGH "), "ABCDEFGH");
    }

    #[test]
    fn labels_are_trimmed_and_bounded() {
        assert_eq!(label("  laptop\n "), "laptop");
        assert_eq!(label(""), "device");
        assert_eq!(label(&"x".repeat(100)).chars().count(), MAX_LABEL_CHARS);
    }
}
