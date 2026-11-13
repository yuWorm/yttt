//! Which node each device relays through.
//!
//! A Host chooses its node and signs that choice (a [`SignedPlacement`]); the
//! control plane stores the newest one and never moves a Host itself. A Client
//! key relays through the node of the one Host it connects to, so a Host and
//! its Clients always share a node. When a Host's node is disabled, leaves the
//! plan or changes its address, the Host gets no ticket until its user
//! chooses again. A device that leaves a node is written to
//! `node_revocations`, which that node keeps refusing even after it restarts.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sqlx::Row as _;
use yttt_relay_ticket::{
    api::{
        DeviceEntry, DeviceRole, NodeChoice, NodePlacement, PlacementNeeded, PlacementReason,
        SwitchNode,
    },
    placement::SignedPlacement,
};

use crate::{
    AppState, Failure,
    auth::{AnySession, Session},
    clamp_i64, now_unix,
};

type Shared = State<Arc<AppState>>;
use crate::db::Tx;

/// How long a Host waits between moves it asks for itself; the console is
/// not held to it.
const SWITCH_COOLDOWN_SECS: u64 = 10 * 60;
/// A device counts as online while a node reported it this recently.
const ONLINE_SECS: u64 = 60;
/// Revocations outlive every ticket (30 days at most).
pub(crate) const REVOCATION_KEEP_SECS: u64 = 31 * 24 * 3600;

pub(crate) fn role_name(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Host => "host",
        DeviceRole::Client => "client",
        DeviceRole::Unknown => "unknown",
    }
}

fn role_of(name: &str) -> DeviceRole {
    match name {
        "host" => DeviceRole::Host,
        "client" => DeviceRole::Client,
        _ => DeviceRole::Unknown,
    }
}

/// The account row, locked so registrations and moves for it run one at a
/// time and its limits hold.
struct Account {
    default_node: String,
    plan_id: String,
    max_devices: i64,
    max_nodes: i64,
}

async fn lock_account(tx: &mut Tx<'_>, account_id: i64) -> Result<Account, Failure> {
    let row = sqlx::query(&format!(
        "SELECT a.node_id, a.plan_id, p.max_devices, p.max_nodes
         FROM accounts a JOIN plans p ON p.id = a.plan_id
         WHERE a.id = $1{}",
        if crate::db::Dialect::of(tx) == crate::db::Dialect::Postgres {
            " FOR UPDATE OF a"
        } else {
            ""
        }
    ))
    .bind(account_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Account {
        default_node: row.get("node_id"),
        plan_id: row.get("plan_id"),
        max_devices: i64::from(row.get::<i32, _>("max_devices")),
        max_nodes: i64::from(row.get::<i32, _>("max_nodes")),
    })
}

struct Device {
    account_id: i64,
    role: DeviceRole,
    node_id: String,
    follows_host: Option<Vec<u8>>,
    node_changed_unix: u64,
}

async fn device(tx: &mut Tx<'_>, endpoint_id: &[u8]) -> Result<Option<Device>, Failure> {
    let row = sqlx::query(
        "SELECT account_id, role, node_id, follows_host, node_changed_unix
         FROM devices WHERE endpoint_id = $1",
    )
    .bind(endpoint_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Device {
        account_id: row.get("account_id"),
        role: role_of(row.get("role")),
        node_id: row.get("node_id"),
        follows_host: row.get("follows_host"),
        node_changed_unix: row.get::<i64, _>("node_changed_unix").max(0) as u64,
    }))
}

/// The node of `host`, which must be a Host on `account_id`.
async fn host_node(tx: &mut Tx<'_>, account_id: i64, host: &[u8]) -> Result<String, Failure> {
    sqlx::query_scalar(
        "SELECT node_id FROM devices WHERE endpoint_id = $1 AND account_id = $2 AND role = 'host'",
    )
    .bind(host)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(Failure::missing("the Host is not on this account"))
}

/// Registers or renames a device. A role, once set, never changes, and a
/// Client key belongs to one Host for good; a device registered by an older
/// version may say what it is once.
pub(crate) async fn register(
    tx: &mut Tx<'_>,
    account_id: i64,
    endpoint_id: &[u8],
    label: &str,
    role: DeviceRole,
    follows_host: Option<[u8; 32]>,
) -> Result<StatusCode, Failure> {
    let account = lock_account(tx, account_id).await?;
    let followed_node = match (role, follows_host) {
        (DeviceRole::Client, Some(host)) => {
            if host[..] == endpoint_id[..] {
                return Err(Failure::bad_request("a device cannot follow itself"));
            }
            Some(host_node(tx, account_id, &host).await?)
        }
        (DeviceRole::Client, None) => {
            return Err(Failure::bad_request(
                "a Client names the Host it connects to",
            ));
        }
        (DeviceRole::Unknown, _) => {
            return Err(Failure::bad_request(
                "a device says whether it is a Host or a Client",
            ));
        }
        (_, Some(_)) => return Err(Failure::bad_request("only a Client follows a Host")),
        (DeviceRole::Host, None) => None,
    };
    let now = now_unix();
    match device(tx, endpoint_id).await? {
        Some(existing) if existing.account_id != account_id => {
            Err(Failure::conflict("the device belongs to another account"))
        }
        Some(existing) => {
            let follows = follows_host.map(|host| host.to_vec());
            let same = existing.role == role && existing.follows_host == follows;
            let upgrade = existing.role == DeviceRole::Unknown;
            if !same && !upgrade && role != DeviceRole::Unknown {
                return Err(Failure::conflict(
                    "the device's role and Host cannot change",
                ));
            }
            sqlx::query("UPDATE devices SET label = $2 WHERE endpoint_id = $1")
                .bind(endpoint_id)
                .bind(label)
                .execute(&mut **tx)
                .await?;
            if upgrade && role != DeviceRole::Unknown {
                sqlx::query(
                    "UPDATE devices SET role = $2, follows_host = $3 WHERE endpoint_id = $1",
                )
                .bind(endpoint_id)
                .bind(role_name(role))
                .bind(follows)
                .execute(&mut **tx)
                .await?;
                if let Some(node) = followed_node {
                    move_devices(tx, Selector::One(endpoint_id), &node, now).await?;
                }
            }
            Ok(StatusCode::OK)
        }
        None => {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE account_id = $1")
                    .bind(account_id)
                    .fetch_one(&mut **tx)
                    .await?;
            if count >= account.max_devices {
                return Err(Failure::forbidden("the plan's device limit is reached"));
            }
            sqlx::query(
                "INSERT INTO devices (endpoint_id, account_id, label, registered_unix, role,
                                      node_id, follows_host)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(endpoint_id)
            .bind(account_id)
            .bind(label)
            .bind(clamp_i64(now))
            .bind(role_name(role))
            .bind(followed_node.unwrap_or(account.default_node))
            .bind(follows_host.map(|host| host.to_vec()))
            .execute(&mut **tx)
            .await?;
            Ok(StatusCode::CREATED)
        }
    }
}

/// What a ticket for a device is for.
pub(crate) enum TicketNode {
    /// A ticket for this node, and the placement the device's Host signed.
    Node(String, SignedPlacement),
    /// No ticket: the Host has to choose a node first.
    Needed(PlacementNeeded),
}

/// The node a ticket for `endpoint_id` is for: the node of the placement its
/// Host (itself, for a Host) signed, while the plan allows that node and its
/// address is the one signed. Nothing here moves a Host.
pub(crate) async fn ticket_node(
    tx: &mut Tx<'_>,
    account_id: i64,
    endpoint_id: &[u8],
    host: Option<[u8; 32]>,
) -> Result<TicketNode, Failure> {
    let account = lock_account(tx, account_id).await?;
    let device = device(tx, endpoint_id)
        .await?
        .filter(|device| device.account_id == account_id)
        .ok_or(Failure::forbidden(
            "the device is not registered to this account",
        ))?;
    if let Some(host) = host
        && (device.role != DeviceRole::Client || device.follows_host.as_deref() != Some(&host[..]))
    {
        return Err(Failure::conflict("the key is not a Client of that Host"));
    }
    let host_key: Vec<u8> = match device.role {
        DeviceRole::Host => endpoint_id.to_vec(),
        DeviceRole::Client => device
            .follows_host
            .clone()
            .ok_or(Failure::conflict("the Client has no Host"))?,
        DeviceRole::Unknown => {
            return Err(Failure::conflict(
                "the device does not say whether it is a Host or a Client; register it again",
            ));
        }
    };
    let host_row = sqlx::query(
        "SELECT node_id, placement FROM devices
         WHERE endpoint_id = $1 AND account_id = $2 AND role = 'host'",
    )
    .bind(&host_key)
    .bind(account_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(Failure::missing("the Host is not on this account"))?;
    let host_node: String = host_row.get("node_id");
    let current = stored_placement(host_row.get("placement"));
    let needed = |reason, current| async move {
        Ok::<_, Failure>(TicketNode::Needed(PlacementNeeded {
            reason,
            current,
            suggested: None,
        }))
    };
    let Some(current) = current else {
        return with_suggestion(tx, &account, needed(PlacementReason::Missing, None).await?).await;
    };
    let allowed = node_allowed(tx, &account.plan_id, &host_node).await?
        && nodes_in_use(tx, account_id, &account.default_node, None, None).await?
            <= account.max_nodes;
    if !allowed || current.placement.node_id != host_node {
        let outcome = needed(PlacementReason::NodeUnavailable, Some(current)).await?;
        return with_suggestion(tx, &account, outcome).await;
    }
    let url: String = sqlx::query_scalar("SELECT url FROM nodes WHERE id = $1")
        .bind(&host_node)
        .fetch_one(&mut **tx)
        .await?;
    if url != current.placement.relay_url {
        let outcome = needed(PlacementReason::AddressChanged, Some(current)).await?;
        return with_suggestion(tx, &account, outcome).await;
    }
    if device.role == DeviceRole::Client && device.node_id != host_node {
        move_devices(tx, Selector::One(endpoint_id), &host_node, now_unix()).await?;
    }
    Ok(TicketNode::Node(host_node, current))
}

/// The placement stored in a `devices.placement` column.
fn stored_placement(column: Option<String>) -> Option<SignedPlacement> {
    column.and_then(|json| serde_json::from_str(&json).ok())
}

/// Adds a node the Host could choose: the account's default if it has room,
/// else any active node of the plan that has room.
async fn with_suggestion(
    tx: &mut Tx<'_>,
    account: &Account,
    outcome: TicketNode,
) -> Result<TicketNode, Failure> {
    let TicketNode::Needed(mut needed) = outcome else {
        return Ok(outcome);
    };
    let rows = sqlx::query(
        "SELECT n.id, n.name, n.region, n.url, n.max_connections, n.last_connected_devices
         FROM nodes n JOIN plan_nodes pn ON pn.node_id = n.id
         WHERE pn.plan_id = $1 AND n.state = 'active'
         ORDER BY CASE WHEN n.id = $2 THEN 0 ELSE 1 END, n.name, n.id",
    )
    .bind(&account.plan_id)
    .bind(&account.default_node)
    .fetch_all(&mut **tx)
    .await?;
    needed.suggested = rows.iter().map(node_choice).find(|choice| !choice.full);
    // An address change keeps the node: suggest that node at its new address.
    if needed.reason == PlacementReason::AddressChanged
        && let Some(current) = &needed.current
    {
        needed.suggested = rows
            .iter()
            .map(node_choice)
            .find(|choice| choice.id == current.placement.node_id)
            .or(needed.suggested);
    }
    Ok(TicketNode::Needed(needed))
}

fn node_choice(row: &crate::db::Row) -> NodeChoice {
    let (load_percent, full) = load(row);
    NodeChoice {
        id: row.get("id"),
        name: row.get("name"),
        region: row.get("region"),
        relay_url: row.get("url"),
        load_percent,
        full,
    }
}

/// Whether `node_id` is active and open to `plan_id`.
async fn node_allowed(tx: &mut Tx<'_>, plan_id: &str, node_id: &str) -> Result<bool, Failure> {
    Ok(sqlx::query_scalar::<_, crate::db::Flag>(
        "SELECT EXISTS (
             SELECT 1 FROM plan_nodes pn JOIN nodes n ON n.id = pn.node_id
             WHERE pn.plan_id = $1 AND pn.node_id = $2 AND n.state = 'active'
         )",
    )
    .bind(plan_id)
    .bind(node_id)
    .fetch_one(&mut **tx)
    .await?
    .0)
}

/// The nodes an account uses: its Hosts' nodes and its default node. With
/// `moving` set, as if that Host were on `to` instead.
async fn nodes_in_use(
    tx: &mut Tx<'_>,
    account_id: i64,
    default_node: &str,
    moving: Option<&[u8]>,
    to: Option<&str>,
) -> Result<i64, Failure> {
    let mut nodes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT node_id FROM devices
         WHERE account_id = $1 AND role = 'host'
           AND ($2 IS NULL OR endpoint_id <> $2)",
    )
    .bind(account_id)
    .bind(moving)
    .fetch_all(&mut **tx)
    .await?;
    nodes.push(default_node.to_owned());
    nodes.extend(to.map(str::to_owned));
    nodes.sort();
    nodes.dedup();
    Ok(nodes.len() as i64)
}

/// Which devices a move takes along.
enum Selector<'a> {
    One(&'a [u8]),
    /// A Host and every Client that follows it.
    HostAndClients(&'a [u8]),
}

/// Moves devices to `to` and has the nodes they leave refuse their tickets.
async fn move_devices(
    tx: &mut Tx<'_>,
    selector: Selector<'_>,
    to: &str,
    now: u64,
) -> Result<(), Failure> {
    let (filter, key) = match selector {
        Selector::One(endpoint) => ("endpoint_id = $1", endpoint),
        Selector::HostAndClients(host) => ("(endpoint_id = $1 OR follows_host = $1)", host),
    };
    let revoke = format!(
        "INSERT INTO node_revocations (node_id, endpoint_id, revoked_unix)
         SELECT node_id, endpoint_id, $3 FROM devices WHERE {filter} AND node_id <> $2"
    );
    let update = format!(
        "UPDATE devices SET node_id = $2, node_changed_unix = $3
         WHERE {filter} AND node_id <> $2"
    );
    for statement in [revoke, update] {
        sqlx::query(&statement)
            .bind(key)
            .bind(to)
            .bind(clamp_i64(now))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Has a device's node, and for a Host its Clients' node, refuse them; used
/// when they are removed from the account.
pub(crate) async fn revoke_removed(tx: &mut Tx<'_>, endpoint_id: &[u8]) -> Result<(), Failure> {
    sqlx::query(
        "INSERT INTO node_revocations (node_id, endpoint_id, revoked_unix)
         SELECT node_id, endpoint_id, $2 FROM devices
         WHERE endpoint_id = $1 OR follows_host = $1",
    )
    .bind(endpoint_id)
    .bind(clamp_i64(now_unix()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Makes `to` the default node of the accounts whose default is `from`,
/// opening `to` to their plans; for administrators. Hosts stay where their
/// users put them: disabling `from` asks each of them to choose again.
pub(crate) async fn move_node(tx: &mut Tx<'_>, from: &str, to: &str) -> Result<u64, Failure> {
    sqlx::query(
        "INSERT INTO plan_nodes (plan_id, node_id)
         SELECT DISTINCT a.plan_id, $2 FROM accounts a
         WHERE a.node_id = $1
            OR a.id IN (SELECT account_id FROM devices WHERE node_id = $1)
         ON CONFLICT DO NOTHING",
    )
    .bind(from)
    .bind(to)
    .execute(&mut **tx)
    .await?;
    let moved = sqlx::query("UPDATE accounts SET node_id = $2 WHERE node_id = $1")
        .bind(from)
        .bind(to)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(moved)
}

/// Opens a new node to every plan; administrators narrow it in the console.
pub(crate) async fn open_node(
    executor: impl sqlx::Executor<'_, Database = sqlx::Any>,
    node_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO plan_nodes (plan_id, node_id) SELECT id, $1 FROM plans WHERE TRUE
         ON CONFLICT DO NOTHING",
    )
    .bind(node_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Opens a new plan to every node.
pub(crate) async fn open_plan(
    executor: impl sqlx::Executor<'_, Database = sqlx::Any>,
    plan_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO plan_nodes (plan_id, node_id) SELECT $1, id FROM nodes WHERE TRUE
         ON CONFLICT DO NOTHING",
    )
    .bind(plan_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// A device as the account's device list shows it.
pub(crate) fn entry(row: &crate::db::Row) -> DeviceEntry {
    let last_seen = row.get::<i64, _>("last_seen_unix").max(0) as u64;
    DeviceEntry {
        endpoint_id: yttt_relay_ticket::hex(&row.get::<Vec<u8>, _>("endpoint_id")),
        label: row.get("label"),
        registered_unix: row.get::<i64, _>("registered_unix") as u64,
        role: role_of(row.get("role")),
        node_id: row.get("node_id"),
        follows_host: row
            .get::<Option<Vec<u8>>, _>("follows_host")
            .map(|host| yttt_relay_ticket::hex(&host)),
        online: now_unix().saturating_sub(last_seen) <= ONLINE_SECS,
    }
}

pub(crate) const DEVICE_COLUMNS: &str =
    "endpoint_id, label, registered_unix, role, node_id, follows_host, last_seen_unix";

/// `GET /v1/nodes`: the nodes the account's plan may use.
pub(crate) async fn list_nodes(
    State(state): Shared,
    session: Session,
) -> Result<Json<Vec<NodeChoice>>, Failure> {
    let rows = sqlx::query(
        "SELECT n.id, n.name, n.region, n.url, n.max_connections, n.last_connected_devices
         FROM nodes n
         JOIN plan_nodes pn ON pn.node_id = n.id
         JOIN accounts a ON a.plan_id = pn.plan_id
         WHERE a.id = $1 AND n.state = 'active'
         ORDER BY n.name, n.id",
    )
    .bind(session.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.iter().map(node_choice).collect()))
}

/// A node's load against its connection limit, and whether it is full.
fn load(row: &crate::db::Row) -> (Option<u32>, bool) {
    let limit: Option<i32> = row.get("max_connections");
    let connected = row
        .get::<Option<i32>, _>("last_connected_devices")
        .unwrap_or(0)
        .max(0);
    match limit {
        Some(limit) if limit > 0 => (
            Some((i64::from(connected) * 100 / i64::from(limit)) as u32),
            connected >= limit,
        ),
        _ => (None, false),
    }
}

/// `PUT /v1/devices/{host}/node`: stores a newer placement the Host signed
/// and moves the Host, with its Clients, to its node. The signature is the
/// only authority: a session that cannot sign for the Host, the console's
/// included, cannot move it. With a plan of one node, the account's default
/// node follows, and its other Hosts are asked to choose again.
pub(crate) async fn switch_node(
    State(state): Shared,
    session: AnySession,
    Path(host): Path<String>,
    Json(request): Json<SwitchNode>,
) -> Result<Json<NodePlacement>, Failure> {
    if session.suspended {
        return Err(Failure::forbidden("the account is suspended"));
    }
    let host = yttt_relay_ticket::parse_key(&host).ok_or_else(Failure::not_found)?;
    let signed = request.placement;
    let placement = signed
        .verify(&host)
        .map_err(|_| Failure::forbidden("the placement is not signed by that Host"))?
        .clone();
    let now = now_unix();
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let account = lock_account(&mut tx, session.account_id).await?;
    let current = device(&mut tx, &host)
        .await?
        .filter(|device| device.account_id == session.account_id && device.role == DeviceRole::Host)
        .ok_or(Failure::missing("the Host is not on this account"))?;
    let (stored, stored_seq): (Option<String>, i64) =
        sqlx::query_as("SELECT placement, placement_seq FROM devices WHERE endpoint_id = $1")
            .bind(&host[..])
            .fetch_one(&mut *tx)
            .await?;
    let stored = stored_placement(stored);
    if placement.seq <= stored_seq.max(0) as u64 {
        return Err(Failure::conflict(
            "the placement is not newer than the one stored",
        ));
    }
    let target = sqlx::query(
        "SELECT n.url, n.max_connections, n.last_connected_devices FROM nodes n
         JOIN plan_nodes pn ON pn.node_id = n.id
         WHERE n.id = $1 AND pn.plan_id = $2 AND n.state = 'active'",
    )
    .bind(&placement.node_id)
    .bind(&account.plan_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Failure::missing("the plan has no such node"))?;
    let relay_url: String = target.get("url");
    if relay_url != placement.relay_url {
        return Err(Failure::conflict("the node has another address"));
    }
    let same_node = stored
        .as_ref()
        .is_some_and(|stored| stored.placement.node_id == placement.node_id)
        && current.node_id == placement.node_id;
    if !same_node && load(&target).1 {
        return Err(Failure::conflict("the node is full"));
    }
    // A Host whose node it may no longer use can choose at once.
    let current_usable =
        stored.is_some() && node_allowed(&mut tx, &account.plan_id, &current.node_id).await?;
    if !same_node
        && current_usable
        && now.saturating_sub(current.node_changed_unix) < SWITCH_COOLDOWN_SECS
    {
        return Err(Failure::conflict(
            "the Host moved recently; try again in a few minutes",
        ));
    }
    if account.max_nodes <= 1 {
        // One node for the account: its default moves with this Host.
        sqlx::query("UPDATE accounts SET node_id = $2 WHERE id = $1")
            .bind(session.account_id)
            .bind(&placement.node_id)
            .execute(&mut *tx)
            .await?;
    } else {
        let used = nodes_in_use(
            &mut tx,
            session.account_id,
            &account.default_node,
            Some(&host),
            Some(&placement.node_id),
        )
        .await?;
        if used > account.max_nodes {
            return Err(Failure::conflict(
                "the plan allows no more nodes at once; move another Host first",
            ));
        }
    }
    let json = serde_json::to_string(&signed).map_err(|_| Failure::bad_request("placement"))?;
    sqlx::query("UPDATE devices SET placement = $2, placement_seq = $3 WHERE endpoint_id = $1")
        .bind(&host[..])
        .bind(json)
        .bind(clamp_i64(placement.seq))
        .execute(&mut *tx)
        .await?;
    move_devices(
        &mut tx,
        Selector::HostAndClients(&host),
        &placement.node_id,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(NodePlacement {
        node_id: placement.node_id,
        relay_url,
    }))
}
