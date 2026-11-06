//! The control plane: accounts, devices, relay tickets and usage.
//!
//! It is a billing service, not an authority over Hosts. It decides who may
//! use which relay node and how fast; it never issues Host identities, never
//! writes a Host's device list, and nothing it signs lets anyone reach a Host.
//! A compromised control plane can deny or meter service, nothing more.

mod admin;
mod api;
mod auth;
mod config;
mod nodes;
mod oauth;
mod plans;
mod web;

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, delete, get, patch, post},
};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use yttt_relay_ticket::TicketSigner;
use zeroize::Zeroizing;

pub use config::{ControlConfig, NodeEntry, PlanConfig, SignupConfig};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("ticket_secret is not 32 bytes of hex")]
    TicketSecret,
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{0}")]
    Refused(&'static str),
    #[error("could not bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },
}

pub(crate) struct AppState {
    pool: PgPool,
    signer: TicketSigner,
    config: ControlConfig,
    limits: auth::Limits,
}

/// A running control plane.
pub struct Control {
    addr: SocketAddr,
    ticket_key: [u8; 32],
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Control {
    pub async fn spawn(config: ControlConfig) -> Result<Self, StartError> {
        let secret = Zeroizing::new(
            yttt_relay_ticket::parse_key(&config.ticket_secret).ok_or(StartError::TicketSecret)?,
        );
        let signer = TicketSigner::from_secret(*secret);
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect(&config.database_url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        seed(&pool, &config).await?;

        let listener =
            TcpListener::bind(config.listen)
                .await
                .map_err(|source| StartError::Bind {
                    addr: config.listen,
                    source,
                })?;
        let addr = listener.local_addr().map_err(|source| StartError::Bind {
            addr: config.listen,
            source,
        })?;
        let ticket_key = signer.public_key();
        let state = Arc::new(AppState {
            pool,
            signer,
            config,
            limits: auth::Limits::default(),
        });
        let probe = tokio::spawn(nodes::probe_loop(state.clone()));
        let router = router(state);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
            probe.abort();
        });
        Ok(Self {
            addr,
            ticket_key,
            stop: Some(stop),
            task,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The key relay nodes must list in `ticket_keys`.
    pub fn ticket_key(&self) -> [u8; 32] {
        self.ticket_key
    }

    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = (&mut self.task).await;
    }
}

fn router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/sessions", post(api::login))
        .route("/v1/oauth/authorize", post(oauth::authorize))
        .route("/v1/oauth/device", post(oauth::start_device))
        .route("/v1/oauth/device/confirm", post(oauth::confirm_device))
        .route("/v1/oauth/token", post(oauth::token))
        .route("/v1/accounts/me", get(api::me))
        .route(
            "/v1/devices",
            post(api::register_device).get(api::list_devices),
        )
        .route("/v1/devices/{endpoint_id}", delete(api::remove_device))
        .route("/v1/tickets", post(api::issue_ticket))
        .route("/v1/nodes/{node_id}/usage", post(api::node_usage))
        .route("/v1/accounts", post(web::sign_up))
        .route("/v1/web/session", post(web::sign_in).delete(web::sign_out))
        .route("/v1/accounts/me/usage", get(web::usage_history))
        .route("/v1/accounts/me/usage/daily", get(web::daily_usage))
        .route("/v1/accounts/me/password", post(web::change_password))
        .route("/v1/admin/overview", get(admin::overview))
        .route(
            "/v1/admin/accounts",
            get(admin::accounts).post(admin::create_account),
        )
        .route(
            "/v1/admin/accounts/{account_id}/password",
            post(admin::reset_password),
        )
        .route("/v1/admin/usage/daily", get(admin::daily_usage))
        .route(
            "/v1/admin/settings",
            get(plans::get_settings).patch(plans::change_settings),
        )
        .route(
            "/v1/admin/accounts/{account_id}",
            get(admin::account).patch(admin::change_account),
        )
        .route(
            "/v1/admin/accounts/{account_id}/devices/{endpoint_id}",
            delete(admin::remove_device),
        )
        .route("/v1/admin/plans", get(plans::list).post(plans::create))
        .route(
            "/v1/admin/plans/{plan_id}",
            patch(plans::change).delete(plans::remove),
        )
        .route("/v1/admin/nodes", get(nodes::list).post(nodes::create))
        .route(
            "/v1/admin/nodes/{node_id}",
            patch(nodes::change).delete(nodes::remove),
        )
        .route(
            "/v1/admin/nodes/{node_id}/enrollment",
            post(nodes::regenerate),
        )
        .route(
            "/v1/admin/nodes/{node_id}/accounts/move/{to}",
            post(nodes::move_accounts),
        )
        .route("/v1/nodes/enroll", post(nodes::enroll))
        .route("/install/relay.sh", get(nodes::install_script))
        .route("/install/bin/{file}", get(nodes::install_binary))
        .route("/install/{*rest}", any(|| async { Failure::not_found() }))
        .route("/v1/admin/audit", get(admin::audit_log))
        // Unknown API paths answer as the API, never with the console page.
        .route("/v1/{*rest}", any(|| async { Failure::not_found() }));
    if state.config.dev_mode {
        router = router
            .route("/v1/dev/accounts", post(api::create_account))
            .route(
                "/v1/dev/accounts/{account_id}/suspension",
                post(api::set_suspended),
            );
    }
    if let Some(root) = state.config.web_root.clone() {
        let index = root.join("index.html");
        router = router.fallback_service(
            tower_http::services::ServeDir::new(root)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    }
    router
        .layer(axum::middleware::map_response(security_headers))
        .with_state(state)
}

/// Locks pages down to this origin: scripts and styles from here only, no
/// framing, no referrer to other sites.
async fn security_headers(mut response: Response) -> Response {
    use axum::http::{HeaderValue, header};
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let html = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    if html {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; font-src 'self' data:; connect-src 'self'; \
                 frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    response
}

/// Makes the account with `email` an administrator; the first administrator
/// can only come from here.
pub async fn promote_admin(config: &ControlConfig, email: &str) -> Result<bool, StartError> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let updated = sqlx::query("UPDATE accounts SET role = 'admin' WHERE email = $1")
        .bind(email.trim().to_ascii_lowercase())
        .execute(&pool)
        .await?
        .rows_affected();
    Ok(updated == 1)
}

/// Creates the first administrator, before any node exists to sign up on.
/// The account joins the sign-up node, which is added as pending when the
/// database has no such node yet; deploying it is the administrator's first
/// job in the console.
pub async fn create_admin(
    config: &ControlConfig,
    email: &str,
    password: &str,
) -> Result<(), StartError> {
    let email = web::normalize_email(email)
        .map_err(|_| StartError::Refused("the email address is invalid"))?;
    web::check_password(password)
        .map_err(|_| StartError::Refused("a password is 8 to 256 characters"))?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    seed(&pool, config).await?;
    let settings = plans::settings(&pool).await?;
    let plan = match settings.signup_plan {
        Some(plan) => plan,
        None => {
            sqlx::query_scalar("SELECT id FROM plans WHERE NOT archived ORDER BY sort, id LIMIT 1")
                .fetch_optional(&pool)
                .await?
                .ok_or(StartError::Refused(
                    "there is no plan; add one under [[plans]]",
                ))?
        }
    };
    let node = settings
        .signup_node
        .or_else(|| config.signup.as_ref().map(|signup| signup.node.clone()))
        .ok_or(StartError::Refused(
            "[signup] names the node for new accounts",
        ))?;
    let password = password.to_owned();
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
        .await
        .expect("hashing never panics");
    let now = clamp_i64(now_unix());
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO nodes (id, url, token_hash, name, state, created_unix)
         VALUES ($1, '', NULL, $1, 'pending', $2)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&node)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE settings SET signup_plan = COALESCE(signup_plan, $1),
                             signup_node = COALESCE(signup_node, $2)",
    )
    .bind(&plan)
    .bind(&node)
    .execute(&mut *tx)
    .await?;
    let created = sqlx::query(
        "INSERT INTO accounts (email, password_hash, plan_id, node_id, created_unix, role)
         VALUES ($1, $2, $3, $4, $5, 'admin')",
    )
    .bind(&email)
    .bind(hash)
    .bind(&plan)
    .bind(&node)
    .bind(now)
    .execute(&mut *tx)
    .await;
    match created {
        Ok(_) => {}
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(StartError::Refused(
                "the email is taken; make it an administrator with promote-admin",
            ));
        }
        Err(error) => return Err(error.into()),
    }
    tx.commit().await?;
    Ok(())
}

/// Seeds an empty database from the configuration: plans, nodes and the
/// sign-up settings. What the database already has is managed in the console
/// and left alone.
async fn seed(pool: &PgPool, config: &ControlConfig) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let has_plans: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM plans)")
        .fetch_one(&mut *tx)
        .await?;
    let now = clamp_i64(now_unix());
    if !has_plans {
        for (sort, plan) in config.plans.iter().enumerate() {
            sqlx::query(
                "INSERT INTO plans (id, name, rate_bps, burst_bytes, monthly_bytes,
                                    throttled_rate_bps, max_devices, sort, created_unix)
                 VALUES ($1, $1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&plan.id)
            .bind(clamp_i32(plan.rate_bps))
            .bind(clamp_i32(plan.burst_bytes))
            .bind(clamp_i64(plan.monthly_bytes))
            .bind(clamp_i32(plan.throttled_rate_bps))
            .bind(clamp_i32(plan.max_devices))
            .bind(i32::try_from(sort).unwrap_or(i32::MAX))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
    }
    for node in &config.nodes {
        sqlx::query(
            "INSERT INTO nodes (id, url, token_hash, name, state, created_unix, enrolled_unix)
             VALUES ($1, $2, $3, $1, 'active', $4, $4)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&node.id)
        .bind(&node.url)
        .bind(sha256(node.token.as_bytes()))
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    // Names that do not exist yet stay empty until the console sets them.
    let signup = config.signup.as_ref();
    sqlx::query(
        "INSERT INTO settings (signup_enabled, signup_plan, signup_node)
         VALUES ($1, (SELECT id FROM plans WHERE id = $2), (SELECT id FROM nodes WHERE id = $3))
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(signup.is_some_and(|signup| signup.enabled))
    .bind(signup.map(|signup| signup.plan.as_str()))
    .bind(signup.map(|signup| signup.node.as_str()))
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// An error the API returns as `{"error": "..."}`.
#[derive(Debug)]
pub(crate) struct Failure(StatusCode, &'static str);

impl Failure {
    pub(crate) const UNAUTHORIZED: Self = Self(StatusCode::UNAUTHORIZED, "unauthorized");
    pub(crate) fn bad_request(reason: &'static str) -> Self {
        Self(StatusCode::BAD_REQUEST, reason)
    }
    pub(crate) fn forbidden(reason: &'static str) -> Self {
        Self(StatusCode::FORBIDDEN, reason)
    }
    pub(crate) fn conflict(reason: &'static str) -> Self {
        Self(StatusCode::CONFLICT, reason)
    }
    pub(crate) fn not_found() -> Self {
        Self(StatusCode::NOT_FOUND, "not found")
    }
    pub(crate) fn unavailable(reason: &'static str) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, reason)
    }
    pub(crate) fn too_many() -> Self {
        Self(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again shortly",
        )
    }
}

impl From<sqlx::Error> for Failure {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "database request failed");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(yttt_relay_ticket::api::ApiError {
                error: self.1.to_owned(),
            }),
        )
            .into_response()
    }
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Year * 100 + month of a Unix time, in UTC.
pub(crate) fn month_of(unix: u64) -> i32 {
    let (year, month, _) = civil_from_days((unix / 86_400) as i64);
    (year * 100 + month) as i32
}

/// Days since the Unix epoch of a Unix time, in UTC.
pub(crate) fn day_of(unix: u64) -> i32 {
    (unix / 86_400) as i32
}

/// The day (since the epoch) the month of `unix` began on.
pub(crate) fn month_start_day(unix: u64) -> i32 {
    let days = (unix / 86_400) as i64;
    let (_, _, day) = civil_from_days(days);
    (days - (day - 1)) as i32
}

/// Year, month and day of a day since the Unix epoch: Howard Hinnant's
/// civil_from_days.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub(crate) fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

pub(crate) fn clamp_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub(crate) fn clamp_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{month_of, month_start_day};

    #[test]
    fn months_follow_the_utc_calendar() {
        assert_eq!(month_of(0), 197_001);
        // 2026-11-06T00:00:00Z
        assert_eq!(month_of(1_793_923_200), 202_611);
        // 2024-02-29T23:59:59Z, then 2024-03-01T00:00:00Z
        assert_eq!(month_of(1_709_251_199), 202_402);
        assert_eq!(month_of(1_709_251_200), 202_403);
        // 2025-12-31T23:59:59Z, then 2026-01-01T00:00:00Z
        assert_eq!(month_of(1_767_225_599), 202_512);
        assert_eq!(month_of(1_767_225_600), 202_601);
    }

    #[test]
    fn a_month_starts_on_its_first_utc_day() {
        // 2026-11-06T12:00:00Z; 2026-11-01 is day 20758.
        assert_eq!(month_start_day(1_793_966_400), 20_758);
        assert_eq!(month_start_day(20_758 * 86_400), 20_758);
        assert_eq!(month_start_day(20_758 * 86_400 - 1), 20_727);
    }
}
