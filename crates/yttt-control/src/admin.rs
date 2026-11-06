//! What administrators do in the console: look at the service, find accounts
//! and change their plan, node, suspension or role, remove devices, and read
//! the audit log. Every change is recorded.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row as _, Transaction};

use crate::{
    AppState, Failure,
    auth::{self, AdminSession},
    clamp_i64, day_of, month_of, month_start_day, now_unix,
    web::{self, DayUsage, MonthUsage, daily, history},
};

type Shared = State<Arc<AppState>>;

/// A node that reported within this long counts as online.
pub(crate) const ONLINE_WINDOW_SECS: u64 = 45;
const MAX_PAGE: i64 = 100;

#[derive(Serialize)]
pub(crate) struct Overview {
    accounts: i64,
    suspended: i64,
    admins: i64,
    /// Accounts created since the start of today (UTC).
    accounts_today: i64,
    month_upstream_bytes: u64,
    today_upstream_bytes: u64,
    /// Devices connected to online nodes at their last report.
    connected_devices: i64,
    nodes: Vec<NodeView>,
}

#[derive(Clone, Serialize)]
pub(crate) struct NodeView {
    pub id: String,
    pub name: String,
    pub url: String,
    /// `pending`, `online`, `offline` or `disabled`.
    pub status: String,
    pub last_report_unix: Option<u64>,
    pub connected_devices: Option<u32>,
    pub version: Option<String>,
    pub reachable: Option<bool>,
    pub accounts: i64,
    /// Devices assigned to the node.
    pub devices: i64,
    pub region: String,
    pub max_connections: Option<u32>,
    /// Bytes relayed through the node this month.
    pub month_bytes: u64,
    /// For a pending node: when its newest enrollment token expires.
    pub enrollment_expires_unix: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct AccountQuery {
    #[serde(default)]
    q: String,
    /// Only accounts on this plan.
    plan: Option<String>,
    /// `active`, `suspended` or `admin`.
    status: Option<String>,
    #[serde(default)]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Serialize)]
pub(crate) struct AccountRow {
    account_id: u64,
    email: String,
    role: String,
    plan: String,
    node_id: String,
    suspended: bool,
    created_unix: u64,
    month_upstream_bytes: u64,
    devices: u32,
}

#[derive(Serialize)]
pub(crate) struct AccountPage {
    total: i64,
    accounts: Vec<AccountRow>,
}

#[derive(Serialize)]
pub(crate) struct AccountDetail {
    account: AccountRow,
    devices: Vec<DeviceRow>,
    usage: Vec<MonthUsage>,
    daily: Vec<DayUsage>,
}

#[derive(Serialize)]
pub(crate) struct DeviceRow {
    endpoint_id: String,
    label: String,
    registered_unix: u64,
}

#[derive(Deserialize)]
pub(crate) struct AccountChange {
    plan: Option<String>,
    node: Option<String>,
    suspended: Option<bool>,
    role: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct NewAccount {
    email: String,
    password: String,
    plan: String,
    node: String,
    #[serde(default = "user_role")]
    role: String,
}

fn user_role() -> String {
    "user".into()
}

#[derive(Deserialize)]
pub(crate) struct NewPassword {
    password: String,
}

#[derive(Serialize)]
pub(crate) struct Created {
    account_id: u64,
}

#[derive(Deserialize)]
pub(crate) struct DaysQuery {
    #[serde(default = "default_days")]
    days: u32,
}

fn default_days() -> u32 {
    30
}

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

#[derive(Serialize)]
pub(crate) struct AuditEntry {
    id: i64,
    admin_email: Option<String>,
    action: String,
    target_account: Option<i64>,
    target_node: Option<String>,
    detail: String,
    at_unix: u64,
}

pub(crate) async fn overview(
    State(state): Shared,
    _admin: AdminSession,
) -> Result<Json<Overview>, Failure> {
    let counts = sqlx::query(
        "SELECT COUNT(*) AS accounts,
                COUNT(*) FILTER (WHERE suspended) AS suspended,
                COUNT(*) FILTER (WHERE role = 'admin') AS admins
         FROM accounts",
    )
    .fetch_one(&state.pool)
    .await?;
    let now = now_unix();
    let month: Option<i64> =
        sqlx::query_scalar("SELECT SUM(upstream_bytes)::BIGINT FROM usage_months WHERE month = $1")
            .bind(month_of(now))
            .fetch_one(&state.pool)
            .await?;
    let today: Option<i64> =
        sqlx::query_scalar("SELECT SUM(upstream_bytes)::BIGINT FROM usage_days WHERE day = $1")
            .bind(day_of(now))
            .fetch_one(&state.pool)
            .await?;
    let accounts_today: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE created_unix >= $1")
            .bind(i64::from(day_of(now)) * 86_400)
            .fetch_one(&state.pool)
            .await?;
    let nodes = nodes(&state).await?;
    Ok(Json(Overview {
        accounts: counts.get("accounts"),
        suspended: counts.get("suspended"),
        admins: counts.get("admins"),
        accounts_today,
        month_upstream_bytes: month.unwrap_or(0) as u64,
        today_upstream_bytes: today.unwrap_or(0) as u64,
        connected_devices: nodes
            .iter()
            .filter(|node| node.status == "online")
            .map(|node| i64::from(node.connected_devices.unwrap_or(0)))
            .sum(),
        nodes,
    }))
}

pub(crate) async fn nodes(state: &AppState) -> Result<Vec<NodeView>, Failure> {
    let rows = sqlx::query(
        "SELECT n.id, n.name, n.url, n.state, n.last_report_unix, n.last_connected_devices,
                n.version, n.reachable, n.region, n.max_connections,
                (SELECT COUNT(*) FROM accounts a WHERE a.node_id = n.id) AS accounts,
                (SELECT COUNT(*) FROM devices d WHERE d.node_id = n.id) AS devices,
                (SELECT COALESCE(SUM(d.upstream_bytes), 0)::BIGINT FROM usage_days d
                  WHERE d.node_id = n.id AND d.day >= $1) AS month_bytes,
                (SELECT MAX(e.expires_unix) FROM node_enrollments e
                  WHERE e.node_id = n.id AND e.used_unix IS NULL) AS enrollment_expires
         FROM nodes n ORDER BY n.id",
    )
    .bind(month_start_day(now_unix()))
    .fetch_all(&state.pool)
    .await?;
    let now = now_unix();
    Ok(rows
        .iter()
        .map(|row| {
            let last: Option<i64> = row.get("last_report_unix");
            let state: String = row.get("state");
            let status = match state.as_str() {
                "disabled" => "disabled",
                "pending" => "pending",
                _ if last
                    .is_some_and(|last| now.saturating_sub(last as u64) <= ONLINE_WINDOW_SECS) =>
                {
                    "online"
                }
                _ => "offline",
            };
            NodeView {
                id: row.get("id"),
                name: row.get("name"),
                url: row.get("url"),
                status: status.into(),
                last_report_unix: last.map(|last| last as u64),
                connected_devices: row
                    .get::<Option<i32>, _>("last_connected_devices")
                    .map(|count| count as u32),
                version: row.get("version"),
                reachable: row.get("reachable"),
                accounts: row.get("accounts"),
                devices: row.get("devices"),
                region: row.get("region"),
                max_connections: row
                    .get::<Option<i32>, _>("max_connections")
                    .map(|limit| limit as u32),
                month_bytes: row.get::<i64, _>("month_bytes") as u64,
                enrollment_expires_unix: row
                    .get::<Option<i64>, _>("enrollment_expires")
                    .map(|at| at as u64),
            }
        })
        .collect())
}

const ACCOUNT_COLUMNS: &str = "a.id, a.email, a.role, a.plan_id, a.node_id, a.suspended,
    a.created_unix, COALESCE(u.upstream_bytes, 0) AS used,
    (SELECT COUNT(*) FROM devices d WHERE d.account_id = a.id) AS devices";

fn account_row(row: &sqlx::postgres::PgRow) -> AccountRow {
    AccountRow {
        account_id: row.get::<i64, _>("id") as u64,
        email: row.get("email"),
        role: row.get("role"),
        plan: row.get("plan_id"),
        node_id: row.get("node_id"),
        suspended: row.get("suspended"),
        created_unix: row.get::<i64, _>("created_unix") as u64,
        month_upstream_bytes: row.get::<i64, _>("used") as u64,
        devices: row.get::<i64, _>("devices") as u32,
    }
}

pub(crate) async fn accounts(
    State(state): Shared,
    _admin: AdminSession,
    Query(query): Query<AccountQuery>,
) -> Result<Json<AccountPage>, Failure> {
    let pattern = format!(
        "%{}%",
        query
            .q
            .trim()
            .to_ascii_lowercase()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let limit = query.limit.clamp(1, MAX_PAGE);
    let offset = query.offset.max(0);
    let status = match query.status.as_deref() {
        None | Some("") => "",
        Some("active") => "AND NOT a.suspended",
        Some("suspended") => "AND a.suspended",
        Some("admin") => "AND a.role = 'admin'",
        Some(_) => return Err(Failure::bad_request("unknown status filter")),
    };
    let filter = format!("a.email LIKE $1 AND ($2::TEXT IS NULL OR a.plan_id = $2) {status}");
    let plan = query.plan.filter(|plan| !plan.is_empty());
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM accounts a WHERE {filter}"))
        .bind(&pattern)
        .bind(&plan)
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts a
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $5
         WHERE {filter} ORDER BY a.id LIMIT $3 OFFSET $4"
    ))
    .bind(&pattern)
    .bind(&plan)
    .bind(limit)
    .bind(offset)
    .bind(month_of(now_unix()))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(AccountPage {
        total,
        accounts: rows.iter().map(account_row).collect(),
    }))
}

pub(crate) async fn account(
    State(state): Shared,
    _admin: AdminSession,
    Path(account_id): Path<i64>,
) -> Result<Json<AccountDetail>, Failure> {
    let row = sqlx::query(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts a
         LEFT JOIN usage_months u ON u.account_id = a.id AND u.month = $2
         WHERE a.id = $1"
    ))
    .bind(account_id)
    .bind(month_of(now_unix()))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(Failure::not_found)?;
    let devices = sqlx::query(
        "SELECT endpoint_id, label, registered_unix FROM devices
         WHERE account_id = $1 ORDER BY registered_unix",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await?
    .iter()
    .map(|row| DeviceRow {
        endpoint_id: yttt_relay_ticket::hex(&row.get::<Vec<u8>, _>("endpoint_id")),
        label: row.get("label"),
        registered_unix: row.get::<i64, _>("registered_unix") as u64,
    })
    .collect();
    Ok(Json(AccountDetail {
        account: account_row(&row),
        devices,
        usage: history(&state, account_id).await?,
        daily: daily(&state, Some(account_id), 30).await?,
    }))
}

pub(crate) async fn change_account(
    State(state): Shared,
    admin: AdminSession,
    Path(account_id): Path<i64>,
    Json(change): Json<AccountChange>,
) -> Result<StatusCode, Failure> {
    if account_id == admin.account_id
        && (change.role.as_deref().is_some_and(|role| role != "admin")
            || change.suspended == Some(true))
    {
        return Err(Failure::forbidden(
            "an administrator cannot demote or suspend themselves",
        ));
    }
    if change
        .role
        .as_deref()
        .is_some_and(|role| role != "admin" && role != "user")
    {
        return Err(Failure::bad_request("a role is user or admin"));
    }
    let mut tx = state.pool.begin().await?;
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Err(Failure::not_found());
    }
    if let Some(plan) = &change.plan {
        let archived: Option<bool> = sqlx::query_scalar("SELECT archived FROM plans WHERE id = $1")
            .bind(plan)
            .fetch_optional(&mut *tx)
            .await?;
        if archived == Some(true) {
            return Err(Failure::bad_request("the plan is archived"));
        }
        set(&mut tx, account_id, "plan_id", plan).await?;
        audit(
            &mut tx,
            admin.account_id,
            "set_plan",
            Some(account_id),
            None,
            plan,
        )
        .await?;
    }
    if let Some(node) = &change.node {
        set(&mut tx, account_id, "node_id", node).await?;
        audit(
            &mut tx,
            admin.account_id,
            "move_account",
            Some(account_id),
            Some(node),
            "",
        )
        .await?;
    }
    if let Some(suspended) = change.suspended {
        sqlx::query("UPDATE accounts SET suspended = $2 WHERE id = $1")
            .bind(account_id)
            .bind(suspended)
            .execute(&mut *tx)
            .await?;
        let action = if suspended { "suspend" } else { "reinstate" };
        audit(
            &mut tx,
            admin.account_id,
            action,
            Some(account_id),
            None,
            "",
        )
        .await?;
    }
    if let Some(role) = &change.role {
        set(&mut tx, account_id, "role", role).await?;
        audit(
            &mut tx,
            admin.account_id,
            "set_role",
            Some(account_id),
            None,
            role,
        )
        .await?;
    }
    tx.commit().await?;
    if change.suspended == Some(true) || change.role.as_deref() == Some("user") {
        auth::end_sessions(&state, account_id, None).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set(
    tx: &mut Transaction<'_, Postgres>,
    account_id: i64,
    column: &'static str,
    value: &str,
) -> Result<(), Failure> {
    let result = sqlx::query(&format!("UPDATE accounts SET {column} = $2 WHERE id = $1"))
        .bind(account_id)
        .bind(value)
        .execute(&mut **tx)
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(error)) if error.is_foreign_key_violation() => {
            Err(Failure::bad_request("unknown plan or node"))
        }
        Err(sqlx::Error::Database(error)) if error.is_check_violation() => {
            Err(Failure::bad_request("invalid value"))
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn remove_device(
    State(state): Shared,
    admin: AdminSession,
    Path((account_id, endpoint_id)): Path<(i64, String)>,
) -> Result<StatusCode, Failure> {
    let key = yttt_relay_ticket::parse_key(&endpoint_id).ok_or_else(Failure::not_found)?;
    let mut tx = state.pool.begin().await?;
    // The device's node, and its Clients' if it is a Host, refuse them.
    sqlx::query(
        "INSERT INTO node_revocations (node_id, endpoint_id, revoked_unix)
         SELECT node_id, endpoint_id, $3 FROM devices
         WHERE (endpoint_id = $1 OR follows_host = $1)
           AND (SELECT account_id FROM devices WHERE endpoint_id = $1) = $2",
    )
    .bind(&key[..])
    .bind(account_id)
    .bind(clamp_i64(now_unix()))
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query("DELETE FROM devices WHERE endpoint_id = $1 AND account_id = $2")
        .bind(&key[..])
        .bind(account_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(Failure::not_found());
    }
    audit(
        &mut tx,
        admin.account_id,
        "remove_device",
        Some(account_id),
        None,
        &endpoint_id,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_account(
    State(state): Shared,
    admin: AdminSession,
    Json(request): Json<NewAccount>,
) -> Result<(StatusCode, Json<Created>), Failure> {
    let email = web::normalize_email(&request.email)?;
    web::check_password(&request.password)?;
    if request.role != "user" && request.role != "admin" {
        return Err(Failure::bad_request("a role is user or admin"));
    }
    let mut tx = state.pool.begin().await?;
    let archived: Option<bool> = sqlx::query_scalar("SELECT archived FROM plans WHERE id = $1")
        .bind(&request.plan)
        .fetch_optional(&mut *tx)
        .await?;
    match archived {
        None => return Err(Failure::bad_request("unknown plan")),
        Some(true) => return Err(Failure::bad_request("the plan is archived")),
        Some(false) => {}
    }
    let node_state: Option<String> = sqlx::query_scalar("SELECT state FROM nodes WHERE id = $1")
        .bind(&request.node)
        .fetch_optional(&mut *tx)
        .await?;
    match node_state.as_deref() {
        None => return Err(Failure::bad_request("unknown node")),
        Some("disabled") => return Err(Failure::bad_request("the node is disabled")),
        Some(_) => {}
    }
    let password = request.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
        .await
        .expect("hashing never panics");
    let created: Result<i64, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO accounts (email, password_hash, plan_id, node_id, role, created_unix)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(&email)
    .bind(hash)
    .bind(&request.plan)
    .bind(&request.node)
    .bind(&request.role)
    .bind(clamp_i64(now_unix()))
    .fetch_one(&mut *tx)
    .await;
    let account_id = match created {
        Ok(id) => id,
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(Failure::conflict("the email is taken"));
        }
        Err(error) => return Err(error.into()),
    };
    audit(
        &mut tx,
        admin.account_id,
        "create_account",
        Some(account_id),
        Some(&request.node),
        &format!("{email}, plan {}, {}", request.plan, request.role),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(Created {
            account_id: account_id as u64,
        }),
    ))
}

/// Sets a new password for someone who lost theirs, and signs them out
/// everywhere. Administrators change their own in the account page.
pub(crate) async fn reset_password(
    State(state): Shared,
    admin: AdminSession,
    Path(account_id): Path<i64>,
    Json(request): Json<NewPassword>,
) -> Result<StatusCode, Failure> {
    if account_id == admin.account_id {
        return Err(Failure::forbidden(
            "change your own password on the account page",
        ));
    }
    web::check_password(&request.password)?;
    let password = request.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
        .await
        .expect("hashing never panics");
    let mut tx = state.pool.begin().await?;
    let updated = sqlx::query("UPDATE accounts SET password_hash = $2 WHERE id = $1")
        .bind(account_id)
        .bind(hash)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(Failure::not_found());
    }
    audit(
        &mut tx,
        admin.account_id,
        "reset_password",
        Some(account_id),
        None,
        "",
    )
    .await?;
    tx.commit().await?;
    auth::end_sessions(&state, account_id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Bytes relayed for everyone, per day.
pub(crate) async fn daily_usage(
    State(state): Shared,
    _admin: AdminSession,
    Query(query): Query<DaysQuery>,
) -> Result<Json<Vec<DayUsage>>, Failure> {
    Ok(Json(daily(&state, None, query.days).await?))
}

pub(crate) async fn audit_log(
    State(state): Shared,
    _admin: AdminSession,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEntry>>, Failure> {
    let rows = sqlx::query(
        "SELECT x.*, a.email AS admin_email FROM admin_actions x
         LEFT JOIN accounts a ON a.id = x.admin_id
         ORDER BY x.at_unix DESC, x.id DESC LIMIT $1",
    )
    .bind(query.limit.clamp(1, 500))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|row| AuditEntry {
                id: row.get("id"),
                admin_email: row.get("admin_email"),
                action: row.get("action"),
                target_account: row.get("target_account"),
                target_node: row.get("target_node"),
                detail: row.get("detail"),
                at_unix: row.get::<i64, _>("at_unix") as u64,
            })
            .collect(),
    ))
}

pub(crate) async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    admin_id: i64,
    action: &str,
    target_account: Option<i64>,
    target_node: Option<&str>,
    detail: &str,
) -> Result<(), Failure> {
    sqlx::query(
        "INSERT INTO admin_actions (admin_id, action, target_account, target_node, detail, at_unix)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(admin_id)
    .bind(action)
    .bind(target_account)
    .bind(target_node)
    .bind(detail)
    .bind(clamp_i64(now_unix()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Serialize)]
pub(crate) struct NodeStats {
    /// Five-minute samples over the last day, oldest first.
    samples: Vec<NodeSample>,
    /// The last 30 days, oldest first and with no gaps.
    days: Vec<NodeDay>,
}

#[derive(Serialize)]
pub(crate) struct NodeSample {
    bucket_unix: u64,
    connections: u32,
    upstream_bytes: u64,
}

#[derive(Serialize)]
pub(crate) struct NodeDay {
    /// Days since the Unix epoch.
    day: i32,
    connections_max: u32,
    upstream_bytes: u64,
}

/// `GET /v1/admin/nodes/{node_id}/stats`: the node's load over a day and a month.
pub(crate) async fn node_stats(
    State(state): Shared,
    _admin: AdminSession,
    Path(node_id): Path<String>,
) -> Result<Json<NodeStats>, Failure> {
    let now = now_unix();
    let samples: Vec<(i64, i32, i64)> = sqlx::query_as(
        "SELECT bucket_unix, connections_max, upstream_bytes FROM node_samples
         WHERE node_id = $1 AND bucket_unix >= $2 ORDER BY bucket_unix",
    )
    .bind(&node_id)
    .bind(clamp_i64(now.saturating_sub(24 * 3600)))
    .fetch_all(&state.pool)
    .await?;
    let today = day_of(now);
    let first = today - 29;
    let peaks: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT day, connections_max FROM node_days WHERE node_id = $1 AND day >= $2",
    )
    .bind(&node_id)
    .bind(first)
    .fetch_all(&state.pool)
    .await?;
    let bytes: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT day, CAST(SUM(upstream_bytes) AS BIGINT) FROM usage_days
         WHERE node_id = $1 AND day >= $2 GROUP BY day",
    )
    .bind(&node_id)
    .bind(first)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(NodeStats {
        samples: samples
            .into_iter()
            .map(|(bucket, connections, upstream)| NodeSample {
                bucket_unix: bucket.max(0) as u64,
                connections: connections.max(0) as u32,
                upstream_bytes: upstream.max(0) as u64,
            })
            .collect(),
        days: (first..=today)
            .map(|day| NodeDay {
                day,
                connections_max: peaks
                    .iter()
                    .find(|(peak_day, _)| *peak_day == day)
                    .map_or(0, |(_, peak)| (*peak).max(0) as u32),
                upstream_bytes: bytes
                    .iter()
                    .find(|(bytes_day, _)| *bytes_day == day)
                    .map_or(0, |(_, bytes)| (*bytes).max(0) as u64),
            })
            .collect(),
    }))
}
