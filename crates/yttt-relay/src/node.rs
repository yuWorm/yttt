//! What a node knows about who is connected and what they may spend.
//!
//! The node never learns what devices say to each other; it only sees opaque
//! datagrams between EndpointIds. A ticket admits a connection, the account's
//! shared bucket paces everything that account sends, and the control plane's
//! standing can change the pace, drop a removed device, or drop the account.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iroh_base::EndpointId;
use iroh_relay::{
    KeyCache,
    server::{
        Access, AccessControl, ClientRequest, ConnectionId, Metrics, clients::Clients,
        streams::Bucket,
    },
};
use parking_lot::Mutex;
use yttt_relay_ticket::{
    TicketVerifier,
    api::{AccountStanding, AccountUsage, Standing, UsageReport},
};

/// Bucket refill granularity; the same as iroh's own per-client limiter.
const REFILL_PERIOD: Duration = Duration::from_millis(100);
/// The smallest rate a bucket accepts at that granularity.
const MIN_RATE_BPS: u32 = 10;
/// Longer than any ticket lives, so a remembered revocation outlasts every
/// ticket it refuses.
const REVOCATION_MEMORY_SECS: u64 = 400 * 24 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rate {
    pub(crate) bytes_per_second: u32,
    pub(crate) burst_bytes: u32,
}

impl Rate {
    fn bucket(self) -> Bucket {
        let rate = self.bytes_per_second.max(MIN_RATE_BPS);
        let burst = self.burst_bytes.max(rate / 10).max(1);
        Bucket::new(i64::from(burst), i64::from(rate), REFILL_PERIOD)
            .expect("a clamped rate always makes a valid bucket")
    }
}

/// One account's meter on this node, shared by all of its connections.
#[derive(Debug)]
pub(crate) struct Account {
    bucket: Mutex<(Rate, Bucket)>,
    ticket_rate: Mutex<Rate>,
    unreported: AtomicU64,
    relayed: AtomicU64,
}

impl Account {
    fn new(rate: Rate) -> Self {
        Self {
            bucket: Mutex::new((rate, rate.bucket())),
            ticket_rate: Mutex::new(rate),
            unreported: AtomicU64::new(0),
            relayed: AtomicU64::new(0),
        }
    }

    /// Counts `bytes` the account sent and says when it may send again.
    pub(crate) fn consume(&self, bytes: usize) -> Result<(), tokio::time::Instant> {
        self.unreported.fetch_add(bytes as u64, Ordering::Relaxed);
        self.relayed.fetch_add(bytes as u64, Ordering::Relaxed);
        self.bucket.lock().1.consume(bytes)
    }

    fn pace(&self, rate: Rate) {
        let mut bucket = self.bucket.lock();
        if bucket.0 != rate {
            *bucket = (rate, rate.bucket());
        }
    }
}

#[derive(Debug)]
struct Live {
    endpoint: EndpointId,
    account: u64,
}

#[derive(Debug, Default)]
struct State {
    accounts: HashMap<u64, Arc<Account>>,
    connections: HashMap<ConnectionId, Live>,
    /// The latest standing the control plane gave each account.
    standings: HashMap<u64, Standing>,
    /// Removed devices, by when they were removed.
    revoked: HashMap<[u8; 32], u64>,
}

impl State {
    fn connections_of(&self, account: u64) -> Vec<(EndpointId, ConnectionId)> {
        self.connections
            .iter()
            .filter(|(_, live)| live.account == account)
            .map(|(id, live)| (live.endpoint, *id))
            .collect()
    }

    fn rate(&self, account: u64, ticket_rate: Rate) -> Rate {
        match self.standings.get(&account) {
            Some(
                Standing::Active {
                    rate_bps,
                    burst_bytes,
                }
                | Standing::Throttled {
                    rate_bps,
                    burst_bytes,
                },
            ) => Rate {
                bytes_per_second: *rate_bps,
                burst_bytes: *burst_bytes,
            },
            Some(Standing::Suspended) | None => ticket_rate,
        }
    }
}

/// A snapshot of one account on this node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub connections: usize,
    /// Every byte the account has sent through this node since it first connected.
    pub relayed_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct Node {
    node_id: String,
    verifier: Option<TicketVerifier>,
    /// Self-hosted mode: admit every endpoint at this rate, each on its own.
    open: Option<Rate>,
    pub(crate) clients: Clients,
    pub(crate) key_cache: KeyCache,
    pub(crate) metrics: Arc<Metrics>,
    state: Mutex<State>,
}

impl Node {
    pub(crate) fn new(
        node_id: String,
        verifier: Option<TicketVerifier>,
        open: Option<Rate>,
    ) -> Self {
        Self {
            node_id,
            verifier,
            open,
            clients: Clients::default(),
            key_cache: KeyCache::new(4096),
            metrics: Arc::new(Metrics::default()),
            state: Mutex::new(State::default()),
        }
    }

    fn admit(&self, request: &ClientRequest) -> Access {
        let endpoint = *request.endpoint_id().as_bytes();
        let ticket = match (request.auth_token(), &self.verifier, self.open) {
            (Some(token), Some(verifier), _) => {
                match verifier.verify(&token, &endpoint, &self.node_id, now_unix()) {
                    Ok(ticket) => ticket,
                    Err(error) => return deny(&error.to_string()),
                }
            }
            (_, _, Some(rate)) => open_ticket(&endpoint, rate),
            _ => return deny("a relay ticket is required"),
        };
        let mut state = self.state.lock();
        if state.standings.get(&ticket.account_id) == Some(&Standing::Suspended) {
            return deny("the account is suspended");
        }
        if state
            .revoked
            .get(&endpoint)
            .is_some_and(|revoked| ticket.issued_unix <= *revoked)
        {
            return deny("the device was removed from its account");
        }
        let ticket_rate = Rate {
            bytes_per_second: ticket.rate_bps,
            burst_bytes: ticket.burst_bytes,
        };
        let rate = state.rate(ticket.account_id, ticket_rate);
        let account = state
            .accounts
            .entry(ticket.account_id)
            .or_insert_with(|| Arc::new(Account::new(rate)))
            .clone();
        *account.ticket_rate.lock() = ticket_rate;
        account.pace(rate);
        state.connections.insert(
            request.connection_id(),
            Live {
                endpoint: request.endpoint_id(),
                account: ticket.account_id,
            },
        );
        Access::Allow
    }

    pub(crate) fn account_of(&self, connection: ConnectionId) -> Option<Arc<Account>> {
        let state = self.state.lock();
        let account = state.connections.get(&connection)?.account;
        state.accounts.get(&account).cloned()
    }

    /// Forgets idle accounts with nothing left to report, and revocations
    /// older than any ticket.
    pub(crate) fn prune(&self, now_unix: u64) {
        let mut state = self.state.lock();
        let State {
            accounts,
            connections,
            standings,
            revoked,
        } = &mut *state;
        accounts.retain(|id, account| {
            account.unreported.load(Ordering::Relaxed) > 0
                || connections.values().any(|live| live.account == *id)
        });
        standings
            .retain(|id, standing| *standing == Standing::Suspended || accounts.contains_key(id));
        revoked.retain(|_, at| now_unix.saturating_sub(*at) < REVOCATION_MEMORY_SECS);
    }

    /// Applies the control plane's standings: new paces, removed devices, and
    /// suspended accounts.
    pub(crate) fn apply(&self, standings: &[AccountStanding]) {
        let now = now_unix();
        let mut dropped = Vec::new();
        {
            let mut state = self.state.lock();
            for AccountStanding {
                account_id,
                standing,
                revoked_endpoints,
            } in standings
            {
                state.standings.insert(*account_id, *standing);
                let revoked: Vec<[u8; 32]> = revoked_endpoints
                    .iter()
                    .filter_map(|key| yttt_relay_ticket::parse_key(key))
                    .collect();
                for key in &revoked {
                    state.revoked.insert(*key, now);
                }
                if *standing == Standing::Suspended {
                    dropped.extend(state.connections_of(*account_id));
                    continue;
                }
                dropped.extend(
                    state
                        .connections_of(*account_id)
                        .into_iter()
                        .filter(|(endpoint, _)| revoked.contains(endpoint.as_bytes())),
                );
                if let Some(account) = state.accounts.get(account_id) {
                    let ticket_rate = *account.ticket_rate.lock();
                    account.pace(state.rate(*account_id, ticket_rate));
                }
            }
        }
        for (endpoint, connection) in dropped {
            self.clients.disconnect(endpoint, Some(connection));
        }
    }

    /// Takes the bytes counted since the last report. A failed report hands
    /// them back with [`Self::restore`].
    pub(crate) fn take_usage(&self) -> UsageReport {
        let state = self.state.lock();
        let mut accounts: Vec<_> = state
            .accounts
            .iter()
            .map(|(id, account)| {
                let mut endpoints: Vec<_> = state
                    .connections_of(*id)
                    .into_iter()
                    .map(|(endpoint, _)| yttt_relay_ticket::hex(endpoint.as_bytes()))
                    .collect();
                endpoints.sort();
                endpoints.dedup();
                AccountUsage {
                    account_id: *id,
                    upstream_bytes: account.unreported.swap(0, Ordering::Relaxed),
                    endpoints,
                }
            })
            .collect();
        for id in state.standings.keys() {
            if !state.accounts.contains_key(id) {
                accounts.push(AccountUsage {
                    account_id: *id,
                    upstream_bytes: 0,
                    endpoints: Vec::new(),
                });
            }
        }
        accounts.sort_by_key(|usage| usage.account_id);
        UsageReport {
            accounts,
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }
    }

    pub(crate) fn restore(&self, report: &UsageReport) {
        let mut state = self.state.lock();
        for usage in &report.accounts {
            if usage.upstream_bytes == 0 {
                continue;
            }
            // An account forgotten since the report keeps its debt in a fresh
            // entry until the next report succeeds.
            let rate = Rate {
                bytes_per_second: MIN_RATE_BPS,
                burst_bytes: 1,
            };
            state
                .accounts
                .entry(usage.account_id)
                .or_insert_with(|| Arc::new(Account::new(rate)))
                .unreported
                .fetch_add(usage.upstream_bytes, Ordering::Relaxed);
        }
    }

    pub(crate) fn snapshot(&self, account: u64) -> Option<AccountSnapshot> {
        let state = self.state.lock();
        let meter = state.accounts.get(&account)?;
        Some(AccountSnapshot {
            connections: state.connections_of(account).len(),
            relayed_bytes: meter.relayed.load(Ordering::Relaxed),
        })
    }
}

/// The node is its own access control, so a connection that ends for any
/// reason leaves the node's books through the disconnect guard.
#[derive(Clone, Debug)]
pub(crate) struct Gate(pub(crate) Arc<Node>);

impl AccessControl for Gate {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        self.0.admit(request)
    }

    fn on_disconnect(&self, _endpoint_id: EndpointId, connection_id: ConnectionId) {
        self.0.state.lock().connections.remove(&connection_id);
    }
}

/// What a self-hosted node admits an endpoint with: an account of its own, so
/// one busy device cannot use up another's rate.
fn open_ticket(endpoint: &[u8; 32], rate: Rate) -> yttt_relay_ticket::Ticket {
    let mut id = [0_u8; 8];
    id.copy_from_slice(&endpoint[..8]);
    yttt_relay_ticket::Ticket {
        account_id: u64::from_be_bytes(id) | (1 << 63),
        endpoint_id: *endpoint,
        node_id: String::new(),
        rate_bps: rate.bytes_per_second,
        burst_bytes: rate.burst_bytes,
        issued_unix: now_unix(),
        expires_unix: u64::MAX,
    }
}

pub(crate) fn open_rate(rate_bps: u32, burst_bytes: u32) -> Rate {
    Rate {
        bytes_per_second: rate_bps,
        burst_bytes,
    }
}

fn deny(reason: &str) -> Access {
    Access::Deny {
        reason: Some(reason.to_owned()),
    }
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
