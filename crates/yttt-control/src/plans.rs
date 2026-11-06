//! Plans and sign-up settings, run from the console. A plan's rates and quota
//! reach the relay nodes with their next usage report, since every report
//! reads them afresh; a lower device limit only stops new registrations.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row as _};

use crate::{AppState, Failure, admin::audit, auth::AdminSession, clamp_i64, now_unix, placement};

type Shared = State<Arc<AppState>>;

const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 500;

#[derive(Serialize)]
pub(crate) struct PlanView {
    id: String,
    name: String,
    description: String,
    rate_bps: u32,
    burst_bytes: u32,
    monthly_bytes: u64,
    throttled_rate_bps: u32,
    max_devices: u32,
    max_nodes: u32,
    /// The nodes the plan may use.
    nodes: Vec<String>,
    archived: bool,
    sort: i32,
    accounts: i64,
}

#[derive(Deserialize)]
pub(crate) struct NewPlan {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    rate_bps: u32,
    burst_bytes: u32,
    monthly_bytes: u64,
    throttled_rate_bps: u32,
    max_devices: u32,
    #[serde(default = "one")]
    max_nodes: u32,
    #[serde(default)]
    sort: i32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
pub(crate) struct PlanChange {
    name: Option<String>,
    description: Option<String>,
    rate_bps: Option<u32>,
    burst_bytes: Option<u32>,
    monthly_bytes: Option<u64>,
    throttled_rate_bps: Option<u32>,
    max_devices: Option<u32>,
    max_nodes: Option<u32>,
    archived: Option<bool>,
    sort: Option<i32>,
    /// Replaces the nodes the plan may use.
    nodes: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Settings {
    pub signup_enabled: bool,
    pub signup_plan: Option<String>,
    pub signup_node: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SettingsChange {
    signup_enabled: Option<bool>,
    signup_plan: Option<String>,
    signup_node: Option<String>,
}

pub(crate) async fn settings(pool: &PgPool) -> Result<Settings, sqlx::Error> {
    let row = sqlx::query("SELECT signup_enabled, signup_plan, signup_node FROM settings")
        .fetch_optional(pool)
        .await?;
    Ok(row.map_or(
        Settings {
            signup_enabled: false,
            signup_plan: None,
            signup_node: None,
        },
        |row| Settings {
            signup_enabled: row.get("signup_enabled"),
            signup_plan: row.get("signup_plan"),
            signup_node: row.get("signup_node"),
        },
    ))
}

pub(crate) async fn list(
    State(state): Shared,
    _admin: AdminSession,
) -> Result<Json<Vec<PlanView>>, Failure> {
    let rows = sqlx::query(
        "SELECT p.*, (SELECT COUNT(*) FROM accounts a WHERE a.plan_id = p.id) AS accounts
         FROM plans p ORDER BY p.archived, p.sort, p.id",
    )
    .fetch_all(&state.pool)
    .await?;
    let open: Vec<(String, String)> =
        sqlx::query_as("SELECT plan_id, node_id FROM plan_nodes ORDER BY node_id")
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(
        rows.iter()
            .map(|row| PlanView {
                id: row.get("id"),
                name: row.get("name"),
                description: row.get("description"),
                rate_bps: row.get::<i32, _>("rate_bps") as u32,
                burst_bytes: row.get::<i32, _>("burst_bytes") as u32,
                monthly_bytes: row.get::<i64, _>("monthly_bytes") as u64,
                throttled_rate_bps: row.get::<i32, _>("throttled_rate_bps") as u32,
                max_devices: row.get::<i32, _>("max_devices") as u32,
                max_nodes: row.get::<i32, _>("max_nodes") as u32,
                nodes: open
                    .iter()
                    .filter(|(plan, _)| *plan == row.get::<String, _>("id"))
                    .map(|(_, node)| node.clone())
                    .collect(),
                archived: row.get("archived"),
                sort: row.get("sort"),
                accounts: row.get("accounts"),
            })
            .collect(),
    ))
}

fn valid_id(id: &str) -> Result<(), Failure> {
    let valid = !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    valid.then_some(()).ok_or(Failure::bad_request(
        "a plan ID is 1 to 32 lowercase letters, digits and hyphens",
    ))
}

fn valid_name(name: &str) -> Result<String, Failure> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(Failure::bad_request("a plan name is 1 to 64 characters"));
    }
    Ok(name.to_owned())
}

fn valid_description(description: &str) -> Result<String, Failure> {
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(Failure::bad_request(
            "a plan description is at most 500 characters",
        ));
    }
    Ok(description.to_owned())
}

/// Rates, burst and device limit fit the database's INTEGER columns.
fn positive(value: u32, reason: &'static str) -> Result<i32, Failure> {
    i32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(Failure::bad_request(reason))
}

pub(crate) async fn create(
    State(state): Shared,
    admin: AdminSession,
    Json(plan): Json<NewPlan>,
) -> Result<(StatusCode, Json<String>), Failure> {
    valid_id(&plan.id)?;
    let name = valid_name(&plan.name)?;
    let description = valid_description(&plan.description)?;
    let rate = positive(plan.rate_bps, "the rate must be positive")?;
    let burst = positive(plan.burst_bytes, "the burst must be positive")?;
    let throttled = positive(
        plan.throttled_rate_bps,
        "the throttled rate must be positive",
    )?;
    let devices = positive(plan.max_devices, "the device limit must be positive")?;
    let mut tx = state.pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO plans (id, name, description, rate_bps, burst_bytes, monthly_bytes,
                            throttled_rate_bps, max_devices, sort, created_unix, max_nodes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&plan.id)
    .bind(&name)
    .bind(&description)
    .bind(rate)
    .bind(burst)
    .bind(clamp_i64(plan.monthly_bytes))
    .bind(throttled)
    .bind(devices)
    .bind(plan.sort)
    .bind(clamp_i64(now_unix()))
    .bind(positive(plan.max_nodes, "the node limit must be positive")?)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(Failure::conflict("a plan with this ID exists"));
        }
        Err(error) => return Err(error.into()),
    }
    placement::open_plan(&mut *tx, &plan.id).await?;
    let detail = format!(
        "{name}: {rate} bps, burst {burst} B, {} B/month, then {throttled} bps, {devices} devices",
        plan.monthly_bytes
    );
    audit(
        &mut tx,
        admin.account_id,
        "create_plan",
        None,
        None,
        &detail,
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(plan.id)))
}

pub(crate) async fn change(
    State(state): Shared,
    admin: AdminSession,
    Path(plan_id): Path<String>,
    Json(change): Json<PlanChange>,
) -> Result<StatusCode, Failure> {
    let mut tx = state.pool.begin().await?;
    let current = sqlx::query("SELECT * FROM plans WHERE id = $1 FOR UPDATE")
        .bind(&plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(Failure::not_found)?;
    if change.archived == Some(true) {
        let signup: Option<String> = sqlx::query_scalar("SELECT signup_plan FROM settings")
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
        if signup.as_deref() == Some(plan_id.as_str()) {
            return Err(Failure::conflict(
                "new accounts get this plan; choose another sign-up plan first",
            ));
        }
    }
    // Each changed field, as `field: before -> after` for the audit log.
    let mut changes = Vec::new();
    let mut note = |field: &str, before: String, after: String| {
        if before != after {
            changes.push(format!("{field}: {before} -> {after}"));
        }
    };
    let name = match &change.name {
        Some(name) => valid_name(name)?,
        None => current.get("name"),
    };
    note("name", current.get("name"), name.clone());
    let description = match &change.description {
        Some(description) => valid_description(description)?,
        None => current.get("description"),
    };
    note(
        "description",
        current.get("description"),
        description.clone(),
    );
    let int = |field: &str, value: Option<u32>, reason| -> Result<i32, Failure> {
        value.map_or_else(|| Ok(current.get(field)), |value| positive(value, reason))
    };
    let rate = int("rate_bps", change.rate_bps, "the rate must be positive")?;
    let burst = int(
        "burst_bytes",
        change.burst_bytes,
        "the burst must be positive",
    )?;
    let throttled = int(
        "throttled_rate_bps",
        change.throttled_rate_bps,
        "the throttled rate must be positive",
    )?;
    let devices = int(
        "max_devices",
        change.max_devices,
        "the device limit must be positive",
    )?;
    let max_nodes = int(
        "max_nodes",
        change.max_nodes,
        "the node limit must be positive",
    )?;
    let monthly = change
        .monthly_bytes
        .map_or_else(|| current.get("monthly_bytes"), clamp_i64);
    let archived = change.archived.unwrap_or_else(|| current.get("archived"));
    let sort = change.sort.unwrap_or_else(|| current.get("sort"));
    for (field, after) in [
        ("rate_bps", rate),
        ("burst_bytes", burst),
        ("throttled_rate_bps", throttled),
        ("max_devices", devices),
        ("max_nodes", max_nodes),
        ("sort", sort),
    ] {
        note(
            field,
            current.get::<i32, _>(field).to_string(),
            after.to_string(),
        );
    }
    note(
        "monthly_bytes",
        current.get::<i64, _>("monthly_bytes").to_string(),
        monthly.to_string(),
    );
    note(
        "archived",
        current.get::<bool, _>("archived").to_string(),
        archived.to_string(),
    );
    if let Some(nodes) = &change.nodes {
        let before: Vec<String> = sqlx::query_scalar(
            "SELECT node_id FROM plan_nodes WHERE plan_id = $1 ORDER BY node_id",
        )
        .bind(&plan_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut after = nodes.clone();
        after.sort();
        after.dedup();
        if after.is_empty() {
            return Err(Failure::bad_request("a plan needs at least one node"));
        }
        if before != after {
            sqlx::query("DELETE FROM plan_nodes WHERE plan_id = $1")
                .bind(&plan_id)
                .execute(&mut *tx)
                .await?;
            for node in &after {
                let added = sqlx::query(
                    "INSERT INTO plan_nodes (plan_id, node_id) SELECT $1, id FROM nodes WHERE id = $2",
                )
                .bind(&plan_id)
                .bind(node)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if added == 0 {
                    return Err(Failure::bad_request("no such node"));
                }
            }
            // Hosts on a node the plan lost return to their default node at
            // their next ticket.
            changes.push(format!(
                "nodes: {} -> {}",
                before.join(","),
                after.join(",")
            ));
        }
    }
    if changes.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    sqlx::query(
        "UPDATE plans SET name = $2, description = $3, rate_bps = $4, burst_bytes = $5,
                          monthly_bytes = $6, throttled_rate_bps = $7, max_devices = $8,
                          archived = $9, sort = $10, max_nodes = $11
         WHERE id = $1",
    )
    .bind(&plan_id)
    .bind(&name)
    .bind(&description)
    .bind(rate)
    .bind(burst)
    .bind(monthly)
    .bind(throttled)
    .bind(devices)
    .bind(archived)
    .bind(sort)
    .bind(max_nodes)
    .execute(&mut *tx)
    .await?;
    let detail = format!("{plan_id}: {}", changes.join("; "));
    audit(
        &mut tx,
        admin.account_id,
        "change_plan",
        None,
        None,
        &detail,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn remove(
    State(state): Shared,
    admin: AdminSession,
    Path(plan_id): Path<String>,
) -> Result<StatusCode, Failure> {
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT (SELECT COUNT(*) FROM accounts a WHERE a.plan_id = p.id) AS accounts,
                EXISTS (SELECT 1 FROM settings s WHERE s.signup_plan = p.id) AS signup
         FROM plans p WHERE p.id = $1 FOR UPDATE",
    )
    .bind(&plan_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(Failure::not_found)?;
    if row.get::<i64, _>("accounts") > 0 {
        return Err(Failure::conflict(
            "accounts use this plan; move them or archive the plan instead",
        ));
    }
    if row.get::<bool, _>("signup") {
        return Err(Failure::conflict(
            "new accounts get this plan; choose another sign-up plan first",
        ));
    }
    sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(&plan_id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        admin.account_id,
        "delete_plan",
        None,
        None,
        &plan_id,
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_settings(
    State(state): Shared,
    _admin: AdminSession,
) -> Result<Json<Settings>, Failure> {
    Ok(Json(settings(&state.pool).await?))
}

pub(crate) async fn change_settings(
    State(state): Shared,
    admin: AdminSession,
    Json(change): Json<SettingsChange>,
) -> Result<StatusCode, Failure> {
    let mut tx = state.pool.begin().await?;
    if let Some(plan) = &change.signup_plan {
        let archived: Option<bool> = sqlx::query_scalar("SELECT archived FROM plans WHERE id = $1")
            .bind(plan)
            .fetch_optional(&mut *tx)
            .await?;
        match archived {
            None => return Err(Failure::bad_request("unknown plan")),
            Some(true) => return Err(Failure::bad_request("the plan is archived")),
            Some(false) => {}
        }
    }
    if let Some(node) = &change.signup_node {
        let node_state: Option<String> =
            sqlx::query_scalar("SELECT state FROM nodes WHERE id = $1")
                .bind(node)
                .fetch_optional(&mut *tx)
                .await?;
        match node_state.as_deref() {
            None => return Err(Failure::bad_request("unknown node")),
            Some("disabled") => return Err(Failure::bad_request("the node is disabled")),
            Some(_) => {}
        }
    }
    sqlx::query(
        "INSERT INTO settings (signup_enabled, signup_plan, signup_node)
         VALUES (COALESCE($1, FALSE), $2, $3)
         ON CONFLICT (id) DO UPDATE SET
             signup_enabled = COALESCE($1, settings.signup_enabled),
             signup_plan = COALESCE($2, settings.signup_plan),
             signup_node = COALESCE($3, settings.signup_node)",
    )
    .bind(change.signup_enabled)
    .bind(&change.signup_plan)
    .bind(&change.signup_node)
    .execute(&mut *tx)
    .await?;
    let mut detail = Vec::new();
    if let Some(enabled) = change.signup_enabled {
        detail.push(format!(
            "sign-up {}",
            if enabled { "open" } else { "closed" }
        ));
    }
    if let Some(plan) = &change.signup_plan {
        detail.push(format!("plan {plan}"));
    }
    if let Some(node) = &change.signup_node {
        detail.push(format!("node {node}"));
    }
    audit(
        &mut tx,
        admin.account_id,
        "change_settings",
        None,
        None,
        &detail.join(", "),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
