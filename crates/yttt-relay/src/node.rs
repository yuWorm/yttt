//! What a node knows about who is connected and what they may spend.
//!
//! The node never learns what devices say to each other; it only sees opaque
//! datagrams between EndpointIds. A ticket admits a connection, the account's
//! shared bucket paces everything that account sends, and the control plane's
//! standing can slow or drop the account at any time.

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
    expires_unix: u64,
}

#[derive(Debug, Default)]
struct State {
    accounts: HashMap<u64, Arc<Account>>,
    connections: HashMap<ConnectionId, Live>,
    /// Only accounts whose standing is not `Active`.
    standings: HashMap<u64, Standing>,
}

impl State {
    fn connections_of(&self, account: u64) -> Vec<(EndpointId, ConnectionId)> {
        self.connections
            .iter()
            .filter(|(_, live)| live.account == account)
            .map(|(id, live)| (live.endpoint, *id))
            .collect()
    }

    fn effective_rate(&self, account: u64, ticket_rate: Rate) -> Rate {
        match self.standings.get(&account) {
            Some(Standing::Throttled {
                rate_bps,
                burst_bytes,
            }) => Rate {
                bytes_per_second: *rate_bps,
                burst_bytes: *burst_bytes,
            },
            _ => ticket_rate,
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
    verifier: TicketVerifier,
    pub(crate) clients: Clients,
    pub(crate) key_cache: KeyCache,
    pub(crate) metrics: Arc<Metrics>,
    state: Mutex<State>,
}

impl Node {
    pub(crate) fn new(node_id: String, verifier: TicketVerifier) -> Self {
        Self {
            node_id,
            verifier,
            clients: Clients::default(),
            key_cache: KeyCache::new(4096),
            metrics: Arc::new(Metrics::default()),
            state: Mutex::new(State::default()),
        }
    }

    fn admit(&self, request: &ClientRequest) -> Access {
        let Some(token) = request.auth_token() else {
            return deny("a relay ticket is required");
        };
        let ticket = match self.verifier.verify(
            &token,
            request.endpoint_id().as_bytes(),
            &self.node_id,
            now_unix(),
        ) {
            Ok(ticket) => ticket,
            Err(error) => return deny(&error.to_string()),
        };
        let mut state = self.state.lock();
        if state.standings.get(&ticket.account_id) == Some(&Standing::Suspended) {
            return deny("the account is suspended");
        }
        let ticket_rate = Rate {
            bytes_per_second: ticket.rate_bps,
            burst_bytes: ticket.burst_bytes,
        };
        let rate = state.effective_rate(ticket.account_id, ticket_rate);
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
                expires_unix: ticket.expires_unix,
            },
        );
        Access::Allow
    }

    pub(crate) fn account_of(&self, connection: ConnectionId) -> Option<Arc<Account>> {
        let state = self.state.lock();
        let account = state.connections.get(&connection)?.account;
        state.accounts.get(&account).cloned()
    }

    /// Drops connections whose ticket has run out, and forgets idle accounts
    /// with nothing left to report.
    pub(crate) fn expire(&self, now_unix: u64) {
        let expired: Vec<_> = {
            let mut state = self.state.lock();
            let State {
                accounts,
                connections,
                standings,
            } = &mut *state;
            accounts.retain(|id, account| {
                standings.contains_key(id)
                    || account.unreported.load(Ordering::Relaxed) > 0
                    || connections.values().any(|live| live.account == *id)
            });
            connections
                .iter()
                .filter(|(_, live)| live.expires_unix <= now_unix)
                .map(|(id, live)| (live.endpoint, *id))
                .collect()
        };
        for (endpoint, connection) in expired {
            self.clients.disconnect(endpoint, Some(connection));
        }
    }

    /// Applies the control plane's standings and drops what is now suspended.
    pub(crate) fn apply(&self, standings: &[AccountStanding]) {
        let mut dropped = Vec::new();
        {
            let mut state = self.state.lock();
            for AccountStanding {
                account_id,
                standing,
            } in standings
            {
                match standing {
                    Standing::Active => {
                        state.standings.remove(account_id);
                    }
                    other => {
                        state.standings.insert(*account_id, *other);
                    }
                }
                if *standing == Standing::Suspended {
                    dropped.extend(state.connections_of(*account_id));
                } else if let Some(account) = state.accounts.get(account_id) {
                    let ticket_rate = *account.ticket_rate.lock();
                    account.pace(state.effective_rate(*account_id, ticket_rate));
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
            .map(|(id, account)| AccountUsage {
                account_id: *id,
                upstream_bytes: account.unreported.swap(0, Ordering::Relaxed),
            })
            .collect();
        for id in state.standings.keys() {
            if !state.accounts.contains_key(id) {
                accounts.push(AccountUsage {
                    account_id: *id,
                    upstream_bytes: 0,
                });
            }
        }
        accounts.sort_by_key(|usage| usage.account_id);
        UsageReport { accounts }
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
