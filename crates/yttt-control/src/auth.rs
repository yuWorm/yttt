//! Who is calling: sessions from devices (bearer) and browsers (cookie),
//! administrators, and limits on guessing passwords and tokens.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use argon2::{
    Argon2, PasswordHash, PasswordHasher as _, PasswordVerifier as _,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderMap, Method, header, request::Parts},
};
use parking_lot::Mutex;
use rand::RngCore as _;
use sqlx::Row as _;

use crate::{AppState, Failure, clamp_i64, now_unix, sha256};

/// Browsers must send this on every request that is not a plain read. A page
/// on another site cannot add it without a CORS preflight, which this server
/// never grants, so a forged cross-site request fails.
pub(crate) const WEB_HEADER: &str = "x-yttt-web";
pub(crate) const COOKIE_SECURE: &str = "__Host-yttt_session";
pub(crate) const COOKIE_PLAIN: &str = "yttt_session";

/// Guesses per source before it must wait, and how fast it earns them back.
const ATTEMPT_BURST: f64 = 10.0;
const ATTEMPTS_PER_MINUTE: f64 = 20.0;
/// Failed logins in a row before an email is locked for a while.
const MAX_EMAIL_FAILURES: u32 = 10;
const EMAIL_LOCK: Duration = Duration::from_secs(15 * 60);
const MAX_TRACKED: usize = 50_000;

/// Verified against when the email is unknown, so a login takes the same time
/// whether or not the account exists.
static DECOY_HASH: LazyLock<String> = LazyLock::new(|| hash_password("decoy password"));

/// An authenticated account. `suspended` accounts reach only what tells them
/// they are suspended.
pub(crate) struct AnySession {
    pub account_id: i64,
    pub token_hash: Vec<u8>,
    pub suspended: bool,
    pub role: String,
}

/// An authenticated account that is not suspended.
pub(crate) struct Session {
    pub account_id: i64,
}

/// An authenticated, not suspended administrator, checked against the
/// database on every request.
pub(crate) struct AdminSession {
    pub account_id: i64,
}

impl FromRequestParts<Arc<AppState>> for AnySession {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Failure> {
        let (token, from_cookie) = match bearer(&parts.headers) {
            Some(token) => (token.to_owned(), false),
            None => (cookie(&parts.headers).ok_or(Failure::UNAUTHORIZED)?, true),
        };
        if from_cookie
            && !matches!(parts.method, Method::GET | Method::HEAD)
            && !has_web_header(&parts.headers)
        {
            return Err(Failure::forbidden(
                "the request is missing the console header",
            ));
        }
        let token_hash = sha256(token.as_bytes());
        let row = sqlx::query(
            "SELECT a.id, a.suspended, a.role FROM sessions s JOIN accounts a ON a.id = s.account_id
             WHERE s.token_hash = $1 AND s.expires_unix > $2",
        )
        .bind(&token_hash)
        .bind(clamp_i64(now_unix()))
        .fetch_optional(&state.pool)
        .await?
        .ok_or(Failure::UNAUTHORIZED)?;
        Ok(Self {
            account_id: row.get("id"),
            token_hash,
            suspended: row.get("suspended"),
            role: row.get("role"),
        })
    }
}

impl Session {
    /// Narrows an already extracted session to one that is not suspended.
    pub(crate) fn from_any(session: &AnySession) -> Result<Self, Failure> {
        if session.suspended {
            return Err(Failure::forbidden("the account is suspended"));
        }
        Ok(Self {
            account_id: session.account_id,
        })
    }
}

impl FromRequestParts<Arc<AppState>> for Session {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Failure> {
        let session = AnySession::from_request_parts(parts, state).await?;
        if session.suspended {
            return Err(Failure::forbidden("the account is suspended"));
        }
        Ok(Self {
            account_id: session.account_id,
        })
    }
}

impl FromRequestParts<Arc<AppState>> for AdminSession {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Failure> {
        let session = AnySession::from_request_parts(parts, state).await?;
        if session.suspended || session.role != "admin" {
            return Err(Failure::forbidden("administrators only"));
        }
        Ok(Self {
            account_id: session.account_id,
        })
    }
}

/// Spends one guess for the caller's address: logins, sign-ups and node
/// enrollment all draw from the same allowance.
pub(crate) struct Attempt;

impl FromRequestParts<Arc<AppState>> for Attempt {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Failure> {
        let source = client_key(&parts.headers, peer(parts), state.config.trusted_proxy);
        if state.limits.attempt(source) {
            Ok(Self)
        } else {
            Err(Failure::too_many())
        }
    }
}

pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

fn cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_SECURE || *name == COOKIE_PLAIN)
        .map(|(_, value)| value.to_owned())
}

pub(crate) fn has_web_header(headers: &HeaderMap) -> bool {
    headers
        .get(WEB_HEADER)
        .is_some_and(|value| value.as_bytes() == b"1")
}

/// The `Set-Cookie` value that opens or ends a browser session.
pub(crate) fn session_cookie(state: &AppState, token: Option<&str>, max_age_secs: u64) -> String {
    let secure = state.config.secure_cookies();
    let name = if secure { COOKIE_SECURE } else { COOKIE_PLAIN };
    let (value, max_age) = match token {
        Some(token) => (token, max_age_secs),
        None => ("", 0),
    };
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

/// Opens a session for `account_id` and returns its token.
pub(crate) async fn open_session(
    state: &AppState,
    account_id: i64,
) -> Result<(String, u64), Failure> {
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
    Ok((token, expires_unix))
}

/// Checks a password against its account, taking the same time for unknown
/// emails, and counting failures per email.
pub(crate) async fn check_login(
    state: &AppState,
    email: &str,
    password: &str,
) -> Result<i64, Failure> {
    if password.len() > 256 || state.limits.email_locked(email) {
        return Err(Failure::UNAUTHORIZED);
    }
    let account = sqlx::query("SELECT id, password_hash FROM accounts WHERE email = $1")
        .bind(email)
        .fetch_optional(&state.pool)
        .await?;
    let (account_id, hash) = match &account {
        Some(row) => (
            Some(row.get::<i64, _>("id")),
            row.get::<String, _>("password_hash"),
        ),
        None => (None, DECOY_HASH.clone()),
    };
    let password = password.to_owned();
    let matches = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .expect("verification never panics");
    match account_id.filter(|_| matches) {
        Some(account_id) => {
            state.limits.email_succeeded(email);
            Ok(account_id)
        }
        None => {
            state.limits.email_failed(email);
            Err(Failure::UNAUTHORIZED)
        }
    }
}

pub(crate) fn hash_password(password: &str) -> String {
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

/// Ends every session of an account, e.g. once it is suspended.
pub(crate) async fn end_sessions(
    state: &AppState,
    account_id: i64,
    except: Option<&[u8]>,
) -> Result<(), Failure> {
    sqlx::query(
        "DELETE FROM sessions WHERE account_id = $1 AND ($2::BYTEA IS NULL OR token_hash <> $2)",
    )
    .bind(account_id)
    .bind(except)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// The caller's address: the TCP peer, or the last `X-Forwarded-For` hop
/// behind one trusted proxy.
pub(crate) fn client_ip(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxy: bool,
) -> IpAddr {
    let forwarded = trusted_proxy
        .then(|| {
            headers
                .get("x-forwarded-for")?
                .to_str()
                .ok()?
                .rsplit(',')
                .next()?
                .trim()
                .parse::<IpAddr>()
                .ok()
        })
        .flatten();
    forwarded
        .or(peer.map(|peer| peer.ip()))
        .unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

/// The caller as rate limits count it: IPv6 callers are grouped by /64, the
/// smallest block one user commonly holds.
pub(crate) fn client_key(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxy: bool,
) -> IpAddr {
    match client_ip(headers, peer, trusted_proxy) {
        IpAddr::V6(ip) => {
            let mut segments = ip.segments();
            segments[4..].fill(0);
            IpAddr::from(segments)
        }
        ip => ip,
    }
}

pub(crate) fn peer(parts: &Parts) -> Option<SocketAddr> {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0)
}

#[derive(Default)]
pub(crate) struct Limits {
    sources: Mutex<HashMap<IpAddr, (f64, Instant)>>,
    emails: Mutex<HashMap<String, (u32, Option<Instant>)>>,
}

impl Limits {
    /// Spends one attempt for `source`; false when it must wait.
    pub(crate) fn attempt(&self, source: IpAddr) -> bool {
        let now = Instant::now();
        let mut sources = self.sources.lock();
        if sources.len() > MAX_TRACKED {
            sources.retain(|_, (tokens, at)| {
                *tokens + at.elapsed().as_secs_f64() * ATTEMPTS_PER_MINUTE / 60.0 < ATTEMPT_BURST
            });
        }
        let (tokens, at) = sources.entry(source).or_insert((ATTEMPT_BURST, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * ATTEMPTS_PER_MINUTE / 60.0)
            .min(ATTEMPT_BURST);
        *at = now;
        if *tokens < 1.0 {
            return false;
        }
        *tokens -= 1.0;
        true
    }

    fn email_locked(&self, email: &str) -> bool {
        self.emails
            .lock()
            .get(email)
            .and_then(|(_, until)| *until)
            .is_some_and(|until| Instant::now() < until)
    }

    fn email_failed(&self, email: &str) {
        let mut emails = self.emails.lock();
        if emails.len() > MAX_TRACKED {
            emails.clear();
        }
        let (failures, until) = emails.entry(email.to_owned()).or_insert((0, None));
        *failures += 1;
        if *failures >= MAX_EMAIL_FAILURES {
            *failures = 0;
            *until = Some(Instant::now() + EMAIL_LOCK);
        }
    }

    fn email_succeeded(&self, email: &str) {
        self.emails.lock().remove(email);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_gets_a_burst_then_waits() {
        let limits = Limits::default();
        let source = IpAddr::from([203, 0, 113, 9]);
        for _ in 0..ATTEMPT_BURST as usize {
            assert!(limits.attempt(source));
        }
        assert!(!limits.attempt(source));
        assert!(limits.attempt(IpAddr::from([203, 0, 113, 10])));
    }

    #[test]
    fn an_email_locks_after_repeated_failures() {
        let limits = Limits::default();
        for _ in 0..MAX_EMAIL_FAILURES {
            assert!(!limits.email_locked("a@example.com"));
            limits.email_failed("a@example.com");
        }
        assert!(limits.email_locked("a@example.com"));
        assert!(!limits.email_locked("b@example.com"));
    }

    #[test]
    fn ipv6_callers_group_by_their_64_and_proxies_count_only_when_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.1, 2001:db8:1:2:aaaa::1".parse().unwrap(),
        );
        let peer = Some("192.0.2.5:4000".parse().unwrap());
        assert_eq!(
            client_key(&headers, peer, false),
            IpAddr::from([192, 0, 2, 5])
        );
        assert_eq!(
            client_key(&headers, peer, true),
            "2001:db8:1:2::".parse::<IpAddr>().unwrap()
        );
    }
}
