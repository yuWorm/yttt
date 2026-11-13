//! Following a paired Host's relay node from a Client.
//!
//! The Host signs where it relays and tells connected devices; the control
//! plane passes the same placement with every ticket. A Client relays only
//! through the node of the newest placement the Host signed, with a ticket for
//! that node, and keeps both in its record so it can connect while the control
//! plane is out of reach.

use std::{
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use parking_lot::Mutex;
use yttt_control_client::{
    ControlError, ControlSession, DeviceKind, PlacedTicket, PlacementNeeded, RelayTicket,
    SignedPlacement, TicketEvents, now_unix,
};
use yttt_transport_iroh::{IrohConnector, IrohEndpoint, RedialHook, RelayTarget};

use crate::{ClientCore, p2p::PairedHost};

/// How long a device that just learned its Host moved waits before dialing
/// it there: the Host's endpoint takes a few seconds to make the new node its
/// home, and a connection made before that ends when it does.
pub const HOST_SETTLE: Duration = Duration::from_secs(5);

/// Called with the record whenever its placement or ticket changes, to store.
pub type RecordChanged = Arc<dyn Fn(&PairedHost) + Send + Sync>;
/// Called with a line for the connection log.
pub type Note = Arc<dyn Fn(&str) + Send + Sync>;

/// Asks for a ticket and the Host's placement, and keeps what is new in
/// `host`. Waits at most `wait`; `None` waits as long as the control plane.
/// Returns the ticket asked for, if one came.
pub async fn refresh_record(
    session: &ControlSession,
    key: &[u8; 32],
    host: &Mutex<PairedHost>,
    wait: Option<Duration>,
    note: &Note,
) -> (Option<RelayTicket>, bool) {
    let kind = DeviceKind::Client {
        host: host.lock().host.0,
    };
    let asked = session.placed_ticket(key, kind);
    let reply = match wait {
        Some(wait) => match tokio::time::timeout(wait, asked).await {
            Ok(reply) => reply,
            Err(_) => {
                note(&format!(
                    "the control plane did not answer within {} s; using what is cached",
                    wait.as_secs()
                ));
                return (None, false);
            }
        },
        None => asked.await,
    };
    match reply {
        Ok(PlacedTicket {
            ticket,
            host_placement,
        }) => {
            let mut host = host.lock();
            let mut changed = false;
            if let Some(placement) = &host_placement {
                match host.accept_placement(placement) {
                    Ok(kept) => changed |= kept,
                    Err(error) => note(&format!("ignored the Host's placement: {error}")),
                }
            }
            if host.ticket.as_ref() != Some(&ticket) {
                host.ticket = Some(ticket.clone());
                changed = true;
            }
            (Some(ticket), changed)
        }
        Err(ControlError::PlacementNeeded(needed)) => {
            note(&placement_needed_line(&needed));
            let mut changed = false;
            if let Some(current) = &needed.current {
                changed = host.lock().accept_placement(current).unwrap_or(false);
            }
            (None, changed)
        }
        Err(error) => {
            note(&format!("no relay ticket: {error}"));
            (None, false)
        }
    }
}

fn placement_needed_line(needed: &PlacementNeeded) -> String {
    format!(
        "the Host relays through no node until its user chooses one ({:?})",
        needed.reason
    )
}

/// Keeps one P2P endpoint and connector on the node the Host chose.
#[derive(Clone)]
pub struct RelayFollower {
    inner: Arc<Inner>,
}

struct Inner {
    session: Option<ControlSession>,
    key: [u8; 32],
    endpoint: IrohEndpoint,
    connector: OnceLock<Weak<IrohConnector>>,
    host: Arc<Mutex<PairedHost>>,
    on_changed: RecordChanged,
    note: Note,
    /// One refresh at a time.
    refreshing: tokio::sync::Mutex<()>,
}

impl RelayFollower {
    /// `session` is the signed-in relay account, if any; `key` this device's
    /// key for the Host, which `endpoint` is bound with.
    pub fn new(
        session: Option<ControlSession>,
        key: [u8; 32],
        endpoint: IrohEndpoint,
        host: Arc<Mutex<PairedHost>>,
        on_changed: RecordChanged,
        note: Note,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                session,
                key,
                endpoint,
                connector: OnceLock::new(),
                host,
                on_changed,
                note,
                refreshing: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// A connector to the Host that asks again where it relays before each
    /// redial, waiting at most `wait` for the control plane.
    pub fn connector(&self, alpn: &[u8], wait: Duration) -> Arc<IrohConnector> {
        let peer = self.inner.host.lock().peer_addr();
        let follower = Arc::downgrade(&self.inner);
        let hook: RedialHook = Arc::new(move || {
            let follower = follower.clone();
            Box::pin(async move {
                let Some(inner) = follower.upgrade() else {
                    return;
                };
                if (RelayFollower { inner }).refresh(Some(wait)).await {
                    tokio::time::sleep(HOST_SETTLE).await;
                }
            })
        });
        let connector = Arc::new(
            IrohConnector::new(self.inner.endpoint.clone(), peer, alpn).with_before_redial(hook),
        );
        let _ = self.inner.connector.set(Arc::downgrade(&connector));
        connector
    }

    /// The record as it stands.
    pub fn host(&self) -> PairedHost {
        self.inner.host.lock().clone()
    }

    /// Asks the control plane, keeps what is new, and points the endpoint and
    /// connector at the Host's node. Whether the Host's relay changed.
    pub async fn refresh(&self, wait: Option<Duration>) -> bool {
        let Some(session) = &self.inner.session else {
            return false;
        };
        let _one = self.inner.refreshing.lock().await;
        let before = self.host().peer_addr().relay_url;
        let (ticket, changed) = refresh_record(
            session,
            &self.inner.key,
            &self.inner.host,
            wait,
            &self.inner.note,
        )
        .await;
        if changed {
            (self.inner.on_changed)(&self.host());
        }
        self.apply(ticket.as_ref()).await;
        self.host().peer_addr().relay_url != before
    }

    /// Takes a placement the Host sent over the session; a newer one with
    /// another node or address moves this device after it.
    pub async fn on_placement(&self, signed: &SignedPlacement) {
        let before = self.host().peer_addr().relay_url;
        let kept = match self.inner.host.lock().accept_placement(signed) {
            Ok(kept) => kept,
            Err(error) => {
                (self.inner.note)(&format!("ignored the Host's placement: {error}"));
                return;
            }
        };
        if !kept {
            return;
        }
        (self.inner.on_changed)(&self.host());
        if self.host().peer_addr().relay_url != before {
            (self.inner.note)(&format!(
                "the Host moved to {}; following it",
                signed.placement.node_id
            ));
            self.refresh(None).await;
        }
    }

    /// Points the endpoint at the Host's node with a usable ticket, or tells
    /// why it cannot, and the connector at the Host's address.
    async fn apply(&self, ticket: Option<&RelayTicket>) {
        let host = self.host();
        match host.relay_target(ticket, now_unix()) {
            Ok(target) => {
                if let Err(error) = self.inner.endpoint.set_relay(&target).await {
                    (self.inner.note)(&format!("could not use the relay: {error}"));
                }
            }
            Err(reason) => (self.inner.note)(&format!("relay unavailable: {reason}")),
        }
        if let Some(connector) = self.inner.connector.get().and_then(Weak::upgrade) {
            connector.set_peer_relay(host.peer_addr().relay_url);
        }
    }

    /// Follows the placements the Host sends on `client`'s connection until
    /// the task is aborted.
    pub fn follow_session(&self, client: &ClientCore) -> tokio::task::JoinHandle<()> {
        let mut placements = client.relay_placement();
        let follower = self.clone();
        tokio::spawn(async move {
            loop {
                let current = placements.borrow_and_update().clone();
                if let Some(current) = current {
                    follower.on_placement(&current).await;
                }
                if placements.changed().await.is_err() {
                    return;
                }
            }
        })
    }

    /// Renews the ticket and checks the Host's node every few minutes while
    /// signed in, even when no ticket came at first.
    pub fn keep_ticket(&self) -> Option<tokio::task::JoinHandle<()>> {
        let session = self.inner.session.clone()?;
        let host = self.host();
        let current = host
            .ticket
            .clone()
            .filter(|ticket| ticket.usable(now_unix()));
        let events = FollowerEvents {
            follower: self.clone(),
            runtime: tokio::runtime::Handle::current(),
        };
        Some(tokio::spawn(yttt_control_client::keep_ticket(
            session,
            self.inner.key,
            DeviceKind::Client { host: host.host.0 },
            self.inner.endpoint.clone(),
            current,
            events,
        )))
    }

    /// The target to bind an endpoint with before any follower exists: the
    /// Host's node with a usable ticket, if the record has them.
    pub fn initial_target(host: &PairedHost) -> Result<RelayTarget, &'static str> {
        host.relay_target(None, now_unix())
    }
}

struct FollowerEvents {
    follower: RelayFollower,
    runtime: tokio::runtime::Handle,
}

impl TicketEvents for FollowerEvents {
    fn renewed(&mut self, ticket: &RelayTicket) {
        let changed = {
            let mut host = self.follower.inner.host.lock();
            let changed = host.ticket.as_ref() != Some(ticket);
            host.ticket = Some(ticket.clone());
            changed
        };
        if changed {
            (self.follower.inner.on_changed)(&self.follower.host());
        }
        // keep_ticket pointed the endpoint at the ticket's node; the
        // connector dials the Host's signed address.
        if let Some(connector) = self.follower.inner.connector.get().and_then(Weak::upgrade) {
            connector.set_peer_relay(self.follower.host().peer_addr().relay_url);
        }
    }

    fn placement(&mut self, placement: &SignedPlacement) {
        let follower = self.follower.clone();
        let placement = placement.clone();
        self.runtime
            .spawn(async move { follower.on_placement(&placement).await });
    }

    fn placement_needed(&mut self, needed: &PlacementNeeded) {
        (self.follower.inner.note)(&placement_needed_line(needed));
    }

    fn failed(&mut self, error: &ControlError) {
        (self.follower.inner.note)(&format!("relay ticket renewal failed: {error}"));
    }
}
