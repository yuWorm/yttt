//! What the web console does for an account holder: sign up, sign in and out
//! with a cookie, change the password, and read the usage history.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row as _;
use yttt_relay_ticket::api::Login;

use crate::{
    AppState, Failure,
    auth::{self, AnySession, Attempt, ConsoleSession, Scope},
    clamp_i64, day_of, month_of, now_unix, plans,
};

type Shared = State<Arc<AppState>>;

const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_BYTES: usize = 256;
const MAX_EMAIL_BYTES: usize = 254;
const HISTORY_MONTHS: usize = 12;
const MAX_DAYS: u32 = 366;
/// How recently an account without a password must have signed in to set
/// one, so a stolen cookie cannot turn into a lasting password.
const SET_PASSWORD_WITHIN_SECS: u64 = 5 * 60;

#[derive(Deserialize)]
pub(crate) struct SignUp {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub(crate) struct WebSession {
    account_id: u64,
    role: String,
}

#[derive(Deserialize)]
pub(crate) struct ChangePassword {
    /// Absent when the account has no password yet.
    #[serde(default)]
    current: Option<String>,
    new: String,
}

#[derive(Serialize)]
pub(crate) struct MonthUsage {
    /// Year * 100 + month.
    month: i32,
    upstream_bytes: u64,
}

pub(crate) async fn sign_up(
    State(state): Shared,
    _attempt: Attempt,
    headers: HeaderMap,
    Json(request): Json<SignUp>,
) -> Result<Response, Failure> {
    require_web(&headers)?;
    let email = normalize_email(&request.email)?;
    check_password(&request.password)?;
    let password = request.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
        .await
        .expect("hashing never panics");
    let account_id = create_account(&state, &email, Some(hash)).await?;
    signed_in(&state, account_id, "user".into(), StatusCode::CREATED).await
}

/// Opens an account through sign-up, on the plan and node the console set
/// for new accounts. `None` for an account that signs in only through GitHub
/// or Google.
pub(crate) async fn create_account(
    state: &AppState,
    email: &str,
    password_hash: Option<String>,
) -> Result<i64, Failure> {
    let settings = plans::settings(&state.pool).await?;
    if !settings.signup_enabled {
        return Err(Failure::not_found());
    }
    let ready: Option<bool> = sqlx::query_scalar(
        "SELECT n.state = 'active' AND NOT p.archived FROM nodes n, plans p
         WHERE n.id = $1 AND p.id = $2",
    )
    .bind(&settings.signup_node)
    .bind(&settings.signup_plan)
    .fetch_optional(&state.pool)
    .await?;
    if ready != Some(true) {
        return Err(Failure::unavailable(
            "sign-up is closed: no relay node is ready",
        ));
    }
    // A full sign-up node hands new accounts to the least busy open node of
    // the plan.
    let node: Option<String> = sqlx::query_scalar(
        "SELECT n.id FROM nodes n JOIN plan_nodes pn ON pn.node_id = n.id
         WHERE pn.plan_id = $2 AND n.state = 'active'
           AND (n.max_connections IS NULL
                OR COALESCE(n.last_connected_devices, 0) < n.max_connections)
         ORDER BY n.id <> $1,
                  CAST(COALESCE(n.last_connected_devices, 0) AS FLOAT8)
                      / COALESCE(n.max_connections, 1000000),
                  n.id
         LIMIT 1",
    )
    .bind(&settings.signup_node)
    .bind(&settings.signup_plan)
    .fetch_optional(&state.pool)
    .await?;
    let node = node.ok_or(Failure::unavailable(
        "sign-up is closed: every relay node is full",
    ))?;
    let created: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO accounts (email, password_hash, plan_id, node_id, created_unix)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(email)
    .bind(password_hash)
    .bind(&settings.signup_plan)
    .bind(&node)
    .bind(clamp_i64(now_unix()))
    .fetch_one(&state.pool)
    .await;
    match created {
        Ok(id) => Ok(id),
        // Tells a caller the email is registered; accepted for a clear message.
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Err(Failure::conflict("the email is taken"))
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn sign_in(
    State(state): Shared,
    _attempt: Attempt,
    headers: HeaderMap,
    Json(request): Json<Login>,
) -> Result<Response, Failure> {
    require_web(&headers)?;
    let email = request.email.trim().to_ascii_lowercase();
    let account_id = auth::check_login(&state, &email, &request.password).await?;
    let role: String = sqlx::query_scalar("SELECT role FROM accounts WHERE id = $1")
        .bind(account_id)
        .fetch_one(&state.pool)
        .await?;
    signed_in(&state, account_id, role, StatusCode::OK).await
}

pub(crate) async fn signed_in(
    state: &AppState,
    account_id: i64,
    role: String,
    status: StatusCode,
) -> Result<Response, Failure> {
    let (token, _) = auth::open_session(state, account_id, Scope::Web).await?;
    let cookie = auth::session_cookie(state, Some(&token), state.config.session_ttl().as_secs());
    let mut response = (
        status,
        Json(WebSession {
            account_id: account_id as u64,
            role,
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("a cookie is a valid header"),
    );
    Ok(response)
}

pub(crate) async fn sign_out(
    State(state): Shared,
    session: AnySession,
) -> Result<Response, Failure> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(&session.token_hash)
        .execute(&state.pool)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::session_cookie(&state, None, 0))
            .expect("a cookie is a valid header"),
    );
    Ok(response)
}

pub(crate) async fn change_password(
    State(state): Shared,
    session: ConsoleSession,
    Json(request): Json<ChangePassword>,
) -> Result<StatusCode, Failure> {
    if session.suspended {
        return Err(Failure::forbidden("the account is suspended"));
    }
    check_password(&request.new)?;
    let (email, has_password): (String, bool) =
        sqlx::query_as("SELECT email, password_hash IS NOT NULL FROM accounts WHERE id = $1")
            .bind(session.account_id)
            .fetch_one(&state.pool)
            .await?;
    if has_password {
        auth::check_login(
            &state,
            &email,
            request.current.as_deref().unwrap_or_default(),
        )
        .await
        .map_err(|_| Failure::forbidden("the current password is wrong"))?;
    } else if !may_set_password(&session) {
        return Err(Failure::forbidden("sign in again to set a password"));
    }
    let new = request.new;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&new))
        .await
        .expect("hashing never panics");
    sqlx::query("UPDATE accounts SET password_hash = $2 WHERE id = $1")
        .bind(session.account_id)
        .bind(hash)
        .execute(&state.pool)
        .await?;
    // Whoever else knew the old password is signed out everywhere.
    auth::end_sessions(&state, session.account_id, Some(&session.token_hash)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether a session signed in recently enough to set the account's first
/// password.
pub(crate) fn may_set_password(session: &ConsoleSession) -> bool {
    now_unix().saturating_sub(session.signed_in_unix) <= SET_PASSWORD_WITHIN_SECS
}

pub(crate) async fn usage_history(
    State(state): Shared,
    session: AnySession,
) -> Result<Json<Vec<MonthUsage>>, Failure> {
    Ok(Json(history(&state, session.account_id).await?))
}

/// Bytes relayed on one UTC day, summed over nodes.
#[derive(Serialize)]
pub(crate) struct DayUsage {
    /// Days since the Unix epoch.
    day: i32,
    upstream_bytes: u64,
}

pub(crate) async fn daily_usage(
    State(state): Shared,
    session: AnySession,
) -> Result<Json<Vec<DayUsage>>, Failure> {
    Ok(Json(daily(&state, Some(session.account_id), 30).await?))
}

/// The last `days` days, oldest first and with no gaps, for one account or
/// for everyone.
pub(crate) async fn daily(
    state: &AppState,
    account_id: Option<i64>,
    days: u32,
) -> Result<Vec<DayUsage>, Failure> {
    let days = days.clamp(1, MAX_DAYS) as i32;
    let today = day_of(now_unix());
    let first = today - days + 1;
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT day, SUM(upstream_bytes)::BIGINT FROM usage_days
         WHERE day >= $1 AND ($2::BIGINT IS NULL OR account_id = $2)
         GROUP BY day",
    )
    .bind(first)
    .bind(account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok((first..=today)
        .map(|day| DayUsage {
            day,
            upstream_bytes: rows
                .iter()
                .find(|(row_day, _)| *row_day == day)
                .map_or(0, |(_, bytes)| *bytes as u64),
        })
        .collect())
}

pub(crate) async fn history(state: &AppState, account_id: i64) -> Result<Vec<MonthUsage>, Failure> {
    let rows = sqlx::query(
        "SELECT month, upstream_bytes FROM usage_months
         WHERE account_id = $1 ORDER BY month DESC LIMIT $2",
    )
    .bind(account_id)
    .bind(HISTORY_MONTHS as i64)
    .fetch_all(&state.pool)
    .await?;
    let current = month_of(now_unix());
    let mut months: Vec<MonthUsage> = rows
        .iter()
        .map(|row| MonthUsage {
            month: row.get("month"),
            upstream_bytes: row.get::<i64, _>("upstream_bytes") as u64,
        })
        .collect();
    if months.first().is_none_or(|first| first.month != current) {
        months.insert(
            0,
            MonthUsage {
                month: current,
                upstream_bytes: 0,
            },
        );
        months.truncate(HISTORY_MONTHS);
    }
    Ok(months)
}

fn require_web(headers: &HeaderMap) -> Result<(), Failure> {
    if auth::has_web_header(headers) {
        Ok(())
    } else {
        Err(Failure::forbidden(
            "the request is missing the console header",
        ))
    }
}

pub(crate) fn normalize_email(email: &str) -> Result<String, Failure> {
    let email = email.trim().to_ascii_lowercase();
    let valid = email.len() <= MAX_EMAIL_BYTES
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    valid
        .then_some(email)
        .ok_or(Failure::bad_request("the email address is invalid"))
}

pub(crate) fn check_password(password: &str) -> Result<(), Failure> {
    if password.chars().count() < MIN_PASSWORD_CHARS || password.len() > MAX_PASSWORD_BYTES {
        return Err(Failure::bad_request("a password is 8 to 256 characters"));
    }
    Ok(())
}
