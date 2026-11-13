//! What a Host or Client says to the control plane: log in, register its own
//! device key, and fetch relay tickets for it.
//!
//! The control plane only sells transport. Nothing here touches which Host a
//! device trusts or which devices a Host admits; those stay end-to-end.

mod sign_in;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use yttt_relay_ticket::{
    DeviceProof, ProofPurpose,
    api::{
        ApiError, DeviceEntry, DeviceList, DeviceProofBody, DeviceRole, Login, RegisterDevice,
        SessionGranted, SwitchNode, TicketOutcome, TicketRequest,
    },
};
use yttt_transport_iroh::{IrohEndpoint, RelayTarget};

pub use sign_in::{BrowserLogin, DeviceLogin};
pub use yttt_relay_ticket::{
    api::{AccountView, NodeChoice, NodePlacement, PlacementNeeded, PlacementReason},
    placement::{PlacementError, SignedPlacement},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const FIRST_RETRY: Duration = Duration::from_secs(30);
const LAST_RETRY: Duration = Duration::from_secs(15 * 60);
/// How often a device asks whether its node changed.
const NODE_CHECK: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("the control plane is unreachable: {0}")]
    Unreachable(String),
    #[error("the control plane refused: {message} ({status})")]
    Refused { status: u16, message: String },
    #[error("the control plane sent an unexpected reply: {0}")]
    Protocol(String),
    #[error("the sign-in was denied")]
    Denied,
    #[error("the sign-in expired; start again")]
    Expired,
    /// No ticket until the Host's user chooses a node.
    #[error("the Host has to choose a relay node ({:?})", .0.reason)]
    PlacementNeeded(Box<PlacementNeeded>),
}

impl ControlError {
    /// The session is no longer valid and the user must log in again.
    pub fn needs_login(&self) -> bool {
        matches!(self, Self::Refused { status: 401, .. })
    }
}

/// What a device key is for, which decides the node it relays through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// A Host's own key; the Host chooses its node.
    Host,
    /// A key for connecting to one Host, on that Host's node.
    Client { host: [u8; 32] },
}

impl DeviceKind {
    fn role(self) -> DeviceRole {
        match self {
            Self::Host => DeviceRole::Host,
            Self::Client { .. } => DeviceRole::Client,
        }
    }

    fn host(self) -> Option<[u8; 32]> {
        match self {
            Self::Host => None,
            Self::Client { host } => Some(host),
        }
    }
}

/// A logged-in account. It holds a bearer token, never the password.
#[derive(Clone, Serialize, Deserialize)]
pub struct ControlSession {
    pub control_url: String,
    pub account_id: u64,
    pub token: String,
}

impl std::fmt::Debug for ControlSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlSession")
            .field("control_url", &self.control_url)
            .field("account_id", &self.account_id)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// A ticket and the placement of the Host it relays to.
#[derive(Clone, Debug)]
pub struct PlacedTicket {
    pub ticket: RelayTicket,
    /// The Host's own for a Host; for a Client, its Host's. Check it against
    /// the Host's key before using it.
    pub host_placement: Option<SignedPlacement>,
}

/// A ticket for one device on its account's relay node.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayTicket {
    pub relay_url: String,
    pub node_id: String,
    pub token: String,
    /// When this device received it, on its own clock.
    pub received_unix: u64,
    pub expires_unix: u64,
}

impl std::fmt::Debug for RelayTicket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelayTicket")
            .field("relay_url", &self.relay_url)
            .field("node_id", &self.node_id)
            .field("token", &"[redacted]")
            .field("received_unix", &self.received_unix)
            .field("expires_unix", &self.expires_unix)
            .finish()
    }
}

impl RelayTicket {
    pub fn target(&self) -> RelayTarget {
        RelayTarget {
            url: self.relay_url.clone(),
            token: Some(self.token.clone()),
        }
    }

    /// Renew once two thirds of the lifetime have passed.
    pub fn renew_at_unix(&self) -> u64 {
        let lifetime = self.expires_unix.saturating_sub(self.received_unix);
        self.expires_unix - lifetime / 3
    }

    /// Whether a relay still admits this ticket, with a minute's margin.
    pub fn usable(&self, now_unix: u64) -> bool {
        now_unix + 60 < self.expires_unix
    }
}

#[derive(Clone)]
pub struct ControlClient {
    http: reqwest::Client,
    base: String,
}

impl ControlClient {
    pub fn new(control_url: &str) -> Result<Self, ControlError> {
        if !(control_url.starts_with("https://") || control_url.starts_with("http://")) {
            return Err(ControlError::Protocol(
                "the control URL must be an http or https URL".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ControlError::Unreachable(error.to_string()))?;
        Ok(Self {
            http,
            base: control_url.trim_end_matches('/').to_owned(),
        })
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<ControlSession, ControlError> {
        let granted: SessionGranted = self
            .send(self.http.post(self.url("/v1/sessions")).json(&Login {
                email: email.to_owned(),
                password: password.to_owned(),
            }))
            .await?;
        Ok(ControlSession {
            control_url: self.base.clone(),
            account_id: granted.account_id,
            token: granted.token,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ControlError> {
        let response = request
            .send()
            .await
            .map_err(|error| ControlError::Unreachable(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let message = response
                .json::<ApiError>()
                .await
                .map_or_else(|_| status.to_string(), |error| error.error);
            return Err(ControlError::Refused {
                status: status.as_u16(),
                message,
            });
        }
        response
            .json()
            .await
            .map_err(|error| ControlError::Protocol(error.to_string()))
    }
}

impl ControlSession {
    fn client(&self) -> Result<ControlClient, ControlError> {
        ControlClient::new(&self.control_url)
    }

    pub async fn account(&self) -> Result<AccountView, ControlError> {
        let client = self.client()?;
        client
            .send(
                client
                    .http
                    .get(client.url("/v1/accounts/me"))
                    .bearer_auth(&self.token),
            )
            .await
    }

    /// Registers this device's key with the account as `kind`; repeating it
    /// only updates the label. A key's kind is fixed once registered.
    pub async fn register_device(
        &self,
        device_secret: &[u8; 32],
        label: &str,
        kind: DeviceKind,
    ) -> Result<DeviceEntry, ControlError> {
        let client = self.client()?;
        client
            .send(
                client
                    .http
                    .post(client.url("/v1/devices"))
                    .bearer_auth(&self.token)
                    .json(&RegisterDevice {
                        label: label.to_owned(),
                        proof: self.proof(device_secret, ProofPurpose::RegisterDevice),
                        role: Some(kind.role()),
                        follows_host: kind.host().map(|host| yttt_relay_ticket::hex(&host)),
                    }),
            )
            .await
    }

    /// The nodes the account's plan may use.
    pub async fn nodes(&self) -> Result<Vec<NodeChoice>, ControlError> {
        let client = self.client()?;
        client
            .send(
                client
                    .http
                    .get(client.url("/v1/nodes"))
                    .bearer_auth(&self.token),
            )
            .await
    }

    /// Stores a placement the Host signed, which moves it and the Clients
    /// that follow it to its node; with a plan of one node, the account's
    /// default node follows.
    pub async fn switch_node(
        &self,
        placement: &SignedPlacement,
    ) -> Result<NodePlacement, ControlError> {
        let client = self.client()?;
        let host = yttt_relay_ticket::hex(&placement.placement.host);
        client
            .send(
                client
                    .http
                    .put(client.url(&format!("/v1/devices/{host}/node")))
                    .bearer_auth(&self.token)
                    .json(&SwitchNode {
                        placement: placement.clone(),
                    }),
            )
            .await
    }

    /// Removes a device from the account; removing one that is not there is
    /// not an error.
    pub async fn remove_device(&self, endpoint_id: &[u8; 32]) -> Result<(), ControlError> {
        let client = self.client()?;
        let response = client
            .http
            .delete(client.url(&format!(
                "/v1/devices/{}",
                yttt_relay_ticket::hex(endpoint_id)
            )))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| ControlError::Unreachable(error.to_string()))?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 404 {
            return Ok(());
        }
        Err(ControlError::Refused {
            status: status.as_u16(),
            message: status.to_string(),
        })
    }

    pub async fn devices(&self) -> Result<DeviceList, ControlError> {
        let client = self.client()?;
        client
            .send(
                client
                    .http
                    .get(client.url("/v1/devices"))
                    .bearer_auth(&self.token),
            )
            .await
    }

    /// A ticket for this key's node: a Host's own, or for a Client the node
    /// of its Host, with the placement that Host signed. A Host that has to
    /// choose a node first gets [`ControlError::PlacementNeeded`].
    pub async fn ticket(
        &self,
        device_secret: &[u8; 32],
        kind: DeviceKind,
    ) -> Result<RelayTicket, ControlError> {
        Ok(self.placed_ticket(device_secret, kind).await?.ticket)
    }

    /// [`Self::ticket`], with the placement the Host signed.
    pub async fn placed_ticket(
        &self,
        device_secret: &[u8; 32],
        kind: DeviceKind,
    ) -> Result<PlacedTicket, ControlError> {
        let client = self.client()?;
        let outcome: TicketOutcome = client
            .send(
                client
                    .http
                    .post(client.url("/v1/tickets"))
                    .bearer_auth(&self.token)
                    .json(&TicketRequest {
                        proof: self.proof(device_secret, ProofPurpose::IssueTicket),
                        host: kind.host().map(|host| yttt_relay_ticket::hex(&host)),
                    }),
            )
            .await?;
        match outcome {
            TicketOutcome::Granted {
                ticket,
                host_placement,
            } => Ok(PlacedTicket {
                ticket: RelayTicket {
                    relay_url: ticket.relay_url,
                    node_id: ticket.node_id,
                    token: ticket.token,
                    received_unix: now_unix(),
                    expires_unix: ticket.expires_unix,
                },
                host_placement,
            }),
            TicketOutcome::PlacementNeeded(needed) => {
                Err(ControlError::PlacementNeeded(Box::new(needed)))
            }
        }
    }

    fn proof(&self, device_secret: &[u8; 32], purpose: ProofPurpose) -> DeviceProofBody {
        DeviceProofBody::from(&DeviceProof::sign(
            device_secret,
            purpose,
            self.account_id,
            now_unix(),
        ))
    }
}

/// What [`keep_ticket`] tells its owner.
pub trait TicketEvents: Send {
    /// A new ticket, already applied to the endpoint.
    fn renewed(&mut self, ticket: &RelayTicket);
    /// The Host's placement, sent with every ticket.
    fn placement(&mut self, _placement: &SignedPlacement) {}
    /// No ticket until the Host's user chooses a node; asked again later.
    fn placement_needed(&mut self, _needed: &PlacementNeeded) {}
    fn failed(&mut self, error: &ControlError);
}

/// [`TicketEvents`] from two closures, for owners that only cache tickets and
/// log errors.
pub fn ticket_events(
    renewed: impl FnMut(&RelayTicket) + Send,
    failed: impl FnMut(&ControlError) + Send,
) -> impl TicketEvents {
    struct Closures<R, F>(R, F);
    impl<R: FnMut(&RelayTicket) + Send, F: FnMut(&ControlError) + Send> TicketEvents
        for Closures<R, F>
    {
        fn renewed(&mut self, ticket: &RelayTicket) {
            (self.0)(ticket);
        }
        fn failed(&mut self, error: &ControlError) {
            (self.1)(error);
        }
    }
    Closures(renewed, failed)
}

/// Renews `current` before it runs out and points `endpoint` at each new
/// ticket, forever; abort the task to stop it. Without a ticket it asks at
/// once. `events` sees each new ticket, e.g. to cache it, each placement and
/// each error.
///
/// iroh keeps the token a relay connection started with, so a renewal on the
/// same URL only reaches relay connections opened afterwards. A long-running
/// endpoint must still be rebuilt before its first ticket expires.
///
/// It also asks every few minutes, so a device follows its node when it
/// moves: the old node refuses it silently, as iroh retries without saying.
pub async fn keep_ticket(
    session: ControlSession,
    device_secret: [u8; 32],
    kind: DeviceKind,
    endpoint: IrohEndpoint,
    mut current: Option<RelayTicket>,
    mut events: impl TicketEvents,
) {
    let mut retry = FIRST_RETRY;
    let mut wait = Duration::ZERO;
    loop {
        if let Some(ticket) = &current {
            wait = Duration::from_secs(
                ticket
                    .renew_at_unix()
                    .saturating_sub(now_unix())
                    .min(NODE_CHECK.as_secs()),
            );
        }
        tokio::time::sleep(wait).await;
        match session.placed_ticket(&device_secret, kind).await {
            Ok(PlacedTicket {
                ticket,
                host_placement,
            }) => {
                retry = FIRST_RETRY;
                if let Some(placement) = &host_placement {
                    events.placement(placement);
                }
                // The same node within its renewal window keeps the ticket it has.
                let fresh = current.as_ref().is_none_or(|current| {
                    ticket.relay_url != current.relay_url || now_unix() >= current.renew_at_unix()
                });
                if fresh {
                    if let Err(error) = endpoint.set_relay(&ticket.target()).await {
                        tracing::warn!(%error, "could not apply a renewed relay ticket");
                    }
                    events.renewed(&ticket);
                    current = Some(ticket);
                }
            }
            Err(ControlError::PlacementNeeded(needed)) => {
                events.placement_needed(&needed);
                // Asked again soon: the user may choose a node at any time.
                current = None;
                wait = FIRST_RETRY;
            }
            Err(error) => {
                events.failed(&error);
                wait = retry;
                retry = (retry * 2).min(LAST_RETRY);
                if current.is_some() {
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_renews_with_a_third_of_its_life_left() {
        let ticket = RelayTicket {
            relay_url: "https://relay.example".into(),
            node_id: "tokyo-1".into(),
            token: "token".into(),
            received_unix: 1_000,
            expires_unix: 4_000,
        };
        assert_eq!(ticket.renew_at_unix(), 3_000);
        assert!(ticket.usable(3_900));
        assert!(!ticket.usable(3_950));
        assert!(!format!("{ticket:?}").contains("token\""));
    }
}
