//! The P2P ingress: paired devices reach this Host over iroh by device key.
//!
//! Only the remote-access manager, driven by local IPC, changes the device list or
//! the open invitation. Network peers can at most present an invitation; a device
//! joins the list only after the Host user confirms its short code.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use hmac::{Hmac, Mac as _};
use parking_lot::{Mutex, RwLock};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::{
    sync::{Semaphore, oneshot, watch},
    task::JoinSet,
};
use yttt_control_client::{ControlSession, RelayTicket};
use yttt_protocol::{
    EndpointKey,
    p2p::{
        DevicePreset, MAX_DEVICE_LABEL_BYTES, MAX_PAIRED_DEVICES, MAX_PAIRING_FAILURES, PAIR_ALPN,
        PAIRING_TTL_MILLIS, PairMac, PairMessage, PairNonce, PairRejectReason, PairTranscript,
        PairedDevice, WORK_ALPN, commit_host_nonce,
    },
    remote_access::{MAX_PENDING_HANDSHAKES, PairingState, RelayAccountStatus},
};
use yttt_transport::{
    Credential, DeviceKeyCredential, IngressKind, TransportStream, receive_pairing, send_pairing,
};
use yttt_transport_iroh::{IrohEndpoint, PeerConnection, PeerGate};
use zeroize::{Zeroize as _, Zeroizing};

use super::ConnectionContext;

const DEVICES_FILE: &str = "devices.json";
const KEY_FILE: &str = "p2p-key.json";
const RELAY_ACCOUNT_FILE: &str = "relay-account.json";
const MAX_STATE_FILE_BYTES: usize = 256 * 1024;
/// How long a pairing peer may take to send its request.
const PAIR_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long an accepted P2P stream has for the session handshake.
const P2P_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize, Deserialize)]
struct DevicesFile {
    schema_version: u32,
    devices: Vec<PairedDevice>,
}

/// The paired devices. Readers see either the old or the new list; a change is
/// persisted before it becomes visible.
pub(crate) struct DeviceRegistry {
    path: PathBuf,
    devices: RwLock<Vec<PairedDevice>>,
}

impl DeviceRegistry {
    pub fn load(root: &Path) -> io::Result<Self> {
        let path = root.join(DEVICES_FILE);
        let devices = match read_private_json::<DevicesFile>(&path)? {
            Some(file) if file.schema_version == 1 => file.devices,
            Some(_) => {
                return Err(io::Error::other("unsupported paired device file version"));
            }
            None => Vec::new(),
        };
        Ok(Self {
            path,
            devices: RwLock::new(devices),
        })
    }

    pub fn contains(&self, device: EndpointKey) -> bool {
        self.devices
            .read()
            .iter()
            .any(|known| known.endpoint_id == device)
    }

    pub fn preset(&self, device: EndpointKey) -> Option<DevicePreset> {
        self.devices
            .read()
            .iter()
            .find(|known| known.endpoint_id == device)
            .map(|known| known.preset)
    }

    pub fn list(&self) -> Vec<PairedDevice> {
        self.devices.read().clone()
    }

    pub fn len(&self) -> usize {
        self.devices.read().len()
    }

    /// Adds or replaces `device`.
    pub fn insert(&self, device: PairedDevice) -> io::Result<()> {
        let mut devices = self.devices.write();
        let mut next = devices.clone();
        next.retain(|known| known.endpoint_id != device.endpoint_id);
        next.push(device);
        self.persist(&next)?;
        *devices = next;
        Ok(())
    }

    /// Removes `device`; the caller then closes its connections.
    pub fn remove(&self, device: EndpointKey) -> io::Result<bool> {
        let mut devices = self.devices.write();
        if !devices.iter().any(|known| known.endpoint_id == device) {
            return Ok(false);
        }
        let next: Vec<_> = devices
            .iter()
            .filter(|known| known.endpoint_id != device)
            .cloned()
            .collect();
        self.persist(&next)?;
        *devices = next;
        Ok(true)
    }

    /// Records activity in memory only; it is saved with the next change.
    pub fn touch(&self, device: EndpointKey) {
        if let Some(known) = self
            .devices
            .write()
            .iter_mut()
            .find(|known| known.endpoint_id == device)
        {
            known.last_seen_millis = Some(super::now_millis());
        }
    }

    fn persist(&self, devices: &[PairedDevice]) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&DevicesFile {
            schema_version: 1,
            devices: devices.to_vec(),
        })
        .map_err(io::Error::other)?;
        super::workspace::atomic_write(&self.path, &bytes, uuid::Uuid::new_v4().as_u128() as u64)
    }
}

#[derive(Serialize, Deserialize)]
struct KeyFile {
    secret: [u8; 32],
}

/// The Host's device key: its iroh EndpointId and its handshake signing key.
pub(crate) fn load_or_create_host_key(root: &Path) -> io::Result<Zeroizing<[u8; 32]>> {
    let path = root.join(KEY_FILE);
    if let Some(mut file) = read_private_json::<KeyFile>(&path)? {
        let secret = Zeroizing::new(file.secret);
        file.secret.zeroize();
        return Ok(secret);
    }
    let mut secret = Zeroizing::new([0_u8; 32]);
    rand::rngs::OsRng.fill_bytes(secret.as_mut());
    let bytes =
        Zeroizing::new(serde_json::to_vec(&KeyFile { secret: *secret }).map_err(io::Error::other)?);
    super::workspace::atomic_write(&path, &bytes, uuid::Uuid::new_v4().as_u128() as u64)?;
    Ok(secret)
}

/// The paid-relay account and the last ticket it got, so a restart while the
/// control plane is down still reaches the relay. It buys transport only.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredRelayAccount {
    pub session: ControlSession,
    #[serde(default)]
    pub ticket: Option<RelayTicket>,
}

pub(crate) fn load_relay_account(root: &Path) -> io::Result<Option<StoredRelayAccount>> {
    read_private_json(&root.join(RELAY_ACCOUNT_FILE))
}

pub(crate) fn save_relay_account(root: &Path, account: &StoredRelayAccount) -> io::Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec(account).map_err(io::Error::other)?);
    super::workspace::atomic_write(
        &root.join(RELAY_ACCOUNT_FILE),
        &bytes,
        uuid::Uuid::new_v4().as_u128() as u64,
    )
}

pub(crate) fn forget_relay_account(root: &Path) -> io::Result<()> {
    match fs::remove_file(root.join(RELAY_ACCOUNT_FILE)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn read_private_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "remote access state must be a private regular file",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    super::validate_user_only_file(path).map_err(io::Error::other)?;
    let bytes = Zeroizing::new(super::workspace::read_bounded_file(
        path,
        MAX_STATE_FILE_BYTES,
    )?);
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

/// What the Host user decided about the device waiting on an invitation.
pub(crate) enum Decision {
    Accept { host_label: String },
    Reject(PairRejectReason),
}

struct Claim {
    device: EndpointKey,
    label: String,
    short_code: u32,
    decision: Option<oneshot::Sender<Decision>>,
}

struct Invitation {
    id: [u8; 16],
    secret: Zeroizing<[u8; 32]>,
    expires_millis: u64,
    failures: u32,
    claim: Option<Claim>,
}

/// The single open invitation, shared by the manager and pairing connections.
#[derive(Default)]
pub(crate) struct PairingSlot {
    invitation: Option<Invitation>,
    compromised: bool,
}

pub(crate) struct NewInvitation {
    pub id: [u8; 16],
    pub secret: Zeroizing<[u8; 32]>,
    pub expires_millis: u64,
}

impl PairingSlot {
    /// Opens a new invitation, withdrawing any earlier one.
    pub fn open(&mut self, now_millis: u64) -> NewInvitation {
        self.close(PairRejectReason::InvitationClosed);
        let mut id = [0_u8; 16];
        let mut secret = Zeroizing::new([0_u8; 32]);
        rand::rngs::OsRng.fill_bytes(&mut id);
        rand::rngs::OsRng.fill_bytes(secret.as_mut());
        let expires_millis = now_millis + PAIRING_TTL_MILLIS;
        self.compromised = false;
        self.invitation = Some(Invitation {
            id,
            secret: secret.clone(),
            expires_millis,
            failures: 0,
            claim: None,
        });
        NewInvitation {
            id,
            secret,
            expires_millis,
        }
    }

    /// Withdraws the invitation and tells a waiting device why.
    pub fn close(&mut self, reason: PairRejectReason) {
        if let Some(mut invitation) = self.invitation.take()
            && let Some(sender) = invitation
                .claim
                .as_mut()
                .and_then(|claim| claim.decision.take())
        {
            let _ = sender.send(Decision::Reject(reason));
        }
    }

    pub fn is_open(&self, now_millis: u64) -> bool {
        self.invitation
            .as_ref()
            .is_some_and(|invitation| invitation.expires_millis > now_millis)
    }

    pub fn state(&self, now_millis: u64) -> Option<PairingState> {
        if self.compromised {
            return Some(PairingState::Compromised);
        }
        let invitation = self
            .invitation
            .as_ref()
            .filter(|invitation| invitation.expires_millis > now_millis)?;
        Some(match &invitation.claim {
            None => PairingState::Open {
                expires_millis: invitation.expires_millis,
            },
            Some(claim) => PairingState::AwaitingConfirmation {
                endpoint_id: claim.device,
                device_label: claim.label.clone(),
                short_code: claim.short_code,
                expires_millis: invitation.expires_millis,
            },
        })
    }

    /// The device waiting for confirmation, if it is `device`.
    pub fn claimed_label(&self, device: EndpointKey) -> Option<String> {
        self.invitation
            .as_ref()?
            .claim
            .as_ref()
            .filter(|claim| claim.device == device)
            .map(|claim| claim.label.clone())
    }

    /// Ends the invitation with the Host user's decision for the waiting device.
    pub fn decide(&mut self, decision: Decision) -> bool {
        let Some(sender) = self
            .invitation
            .take()
            .and_then(|mut invitation| invitation.claim.take())
            .and_then(|mut claim| claim.decision.take())
        else {
            return false;
        };
        sender.send(decision).is_ok()
    }
}

/// Admits paired devices to work sessions, and any peer to pairing while an
/// invitation is open.
pub(crate) struct HostGate {
    pub devices: Arc<DeviceRegistry>,
    pub pairing: Arc<Mutex<PairingSlot>>,
}

impl PeerGate for HostGate {
    fn admit(&self, peer: EndpointKey, alpn: &[u8]) -> bool {
        match alpn {
            WORK_ALPN => self.devices.contains(peer),
            PAIR_ALPN => self.pairing.lock().is_open(super::now_millis()),
            _ => false,
        }
    }
}

/// What every connection of the P2P ingress shares.
#[derive(Clone)]
pub(crate) struct P2pShared {
    pub endpoint: IrohEndpoint,
    pub secret: Arc<Zeroizing<[u8; 32]>>,
    pub devices: Arc<DeviceRegistry>,
    pub pairing: Arc<Mutex<PairingSlot>>,
    /// Remote streams of both ingresses.
    pub streams: Arc<Semaphore>,
    pub sequence: Arc<AtomicU64>,
}

pub(crate) async fn accept_loop(shared: P2pShared, mut context: ConnectionContext) {
    context.identity.ingress = IngressKind::P2pWork;
    context.identity.credential_generation = 0;
    let handshakes = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
    let mut stop = context.stop.clone();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            _ = connections.join_next(), if !connections.is_empty() => {}
            accepted = shared.endpoint.accept() => {
                let Some(connection) = accepted else { break };
                let shared = shared.clone();
                let context = context.clone();
                let handshakes = handshakes.clone();
                connections.spawn(async move {
                    if connection.alpn() == PAIR_ALPN {
                        serve_pairing_connection(connection, &shared, &context).await;
                    } else {
                        serve_work_connection(connection, &shared, context, handshakes).await;
                    }
                });
            }
        }
    }
    shared.endpoint.close().await;
    connections.shutdown().await;
}

async fn serve_work_connection(
    connection: PeerConnection,
    shared: &P2pShared,
    context: ConnectionContext,
    handshakes: Arc<Semaphore>,
) {
    let device = connection.peer();
    let mut sessions = JoinSet::new();
    let mut stop = context.stop.clone();
    loop {
        let stream = tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } continue; }
            _ = sessions.join_next(), if !sessions.is_empty() => continue,
            // The transport re-checks the gate, so a revoked device gets no stream.
            stream = connection.accept_stream() => match stream { Some(stream) => stream, None => break },
        };
        let (Ok(pending), Ok(slot)) = (
            handshakes.clone().try_acquire_owned(),
            shared.streams.clone().try_acquire_owned(),
        ) else {
            drop(stream);
            continue;
        };
        let Some(preset) = shared.devices.preset(device) else {
            break;
        };
        let mut session = context.clone();
        session.identity.connection_sequence = shared.sequence.fetch_add(1, Ordering::Relaxed);
        session.handshake_permit = Some(Arc::new(Mutex::new(Some(pending))));
        session.handshake_deadline = Some(tokio::time::Instant::now() + P2P_HANDSHAKE_TIMEOUT);
        session.device = Some(super::PairedPeer {
            key: device,
            preset,
            devices: shared.devices.clone(),
        });
        let credential = Arc::new(Credential::DeviceKey(Box::new(
            DeviceKeyCredential::new(**shared.secret).with_peer(device),
        )));
        sessions.spawn(async move {
            let _slot = slot;
            let _ = super::serve_connection(stream, credential, session).await;
        });
    }
    connection.close();
    sessions.shutdown().await;
}

async fn serve_pairing_connection(
    connection: PeerConnection,
    shared: &P2pShared,
    context: &ConnectionContext,
) {
    // The Host speaks first, and a QUIC peer only learns of a stream once its
    // opener writes, so the Host opens the pairing stream.
    let Ok(mut stream) = connection.open_stream().await else {
        return;
    };
    let mut stop = context.stop.clone();
    tokio::select! {
        _ = stop.changed() => {}
        _ = pair(&mut stream, connection.peer(), shared, context) => {}
    }
    // Let the last message reach the device before the connection closes.
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn pair(
    stream: &mut TransportStream,
    device: EndpointKey,
    shared: &P2pShared,
    context: &ConnectionContext,
) {
    let reject = async |stream: &mut TransportStream, reason| {
        let _ = send_pairing(stream, &PairMessage::Rejected(reason)).await;
    };
    let mut host_nonce = PairNonce([0; 32]);
    rand::rngs::OsRng.fill_bytes(&mut host_nonce.0);
    let commit = commit_host_nonce(&host_nonce);
    if send_pairing(
        stream,
        &PairMessage::Hello {
            host_nonce_commit: commit,
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let request = tokio::time::timeout(PAIR_REQUEST_TIMEOUT, receive_pairing(stream)).await;
    let Ok(Ok(PairMessage::Request {
        invitation_id,
        device_label,
        client_nonce,
        mac,
    })) = request
    else {
        return reject(stream, PairRejectReason::InvalidMessage).await;
    };
    if device_label.is_empty() || device_label.len() > MAX_DEVICE_LABEL_BYTES {
        return reject(stream, PairRejectReason::InvalidMessage).await;
    }
    let host = shared.endpoint.id();
    let transcript = PairTranscript {
        invitation_id: &invitation_id,
        host: &host,
        client: &device,
        environment_id: &context.identity.environment_id,
        profile_id: &context.profile_id,
        host_nonce_commit: &commit,
        client_nonce: &client_nonce,
        device_label: &device_label,
    };
    let claimed = claim_invitation(
        shared,
        device,
        &device_label,
        &invitation_id,
        &transcript,
        &mac,
        &host_nonce,
    );
    let decision = match claimed {
        Ok(decision) => decision,
        Err(reason) => return reject(stream, reason).await,
    };
    if send_pairing(stream, &PairMessage::Reveal { host_nonce })
        .await
        .is_err()
    {
        shared
            .pairing
            .lock()
            .close(PairRejectReason::InvitationClosed);
        return;
    }
    let message = match decision.await {
        Ok(Decision::Accept { host_label }) => PairMessage::Accepted {
            environment_id: context.identity.environment_id.clone(),
            profile_id: context.profile_id.clone(),
            host_label,
        },
        Ok(Decision::Reject(reason)) => PairMessage::Rejected(reason),
        Err(_) => PairMessage::Rejected(PairRejectReason::InvitationClosed),
    };
    let _ = send_pairing(stream, &message).await;
}

/// Checks a pairing request against the open invitation and, if it is the first
/// valid one, records the device as waiting for the Host user.
fn claim_invitation(
    shared: &P2pShared,
    device: EndpointKey,
    device_label: &str,
    invitation_id: &[u8; 16],
    transcript: &PairTranscript<'_>,
    mac: &PairMac,
    host_nonce: &PairNonce,
) -> Result<oneshot::Receiver<Decision>, PairRejectReason> {
    let now = super::now_millis();
    let mut slot = shared.pairing.lock();
    let Some(invitation) = slot
        .invitation
        .as_mut()
        .filter(|invitation| invitation.id == *invitation_id && invitation.expires_millis > now)
    else {
        return Err(PairRejectReason::InvitationClosed);
    };
    if !mac_matches(&invitation.secret, &transcript.request_bytes(), mac) {
        invitation.failures += 1;
        if invitation.failures >= MAX_PAIRING_FAILURES {
            slot.close(PairRejectReason::InvitationClosed);
        }
        return Err(PairRejectReason::AuthenticationFailed);
    }
    if invitation.claim.is_some() {
        // Two holders of one code: whoever photographed it may be either.
        slot.close(PairRejectReason::InvitationClosed);
        slot.compromised = true;
        return Err(PairRejectReason::InvitationClosed);
    }
    if shared.devices.len() >= MAX_PAIRED_DEVICES && !shared.devices.contains(device) {
        return Err(PairRejectReason::DeviceLimitReached);
    }
    let (sender, decision) = oneshot::channel();
    invitation.claim = Some(Claim {
        device,
        label: device_label.to_string(),
        short_code: transcript.short_code(host_nonce),
        decision: Some(sender),
    });
    Ok(decision)
}

fn mac_matches(secret: &[u8; 32], payload: &[u8], mac: &PairMac) -> bool {
    let mut expected = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts 32-byte keys");
    expected.update(payload);
    expected.verify_slice(&mac.0).is_ok()
}

/// Stops the P2P ingress when dropped.
pub(crate) struct P2pGeneration {
    pub shared: P2pShared,
    pub cancel: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
    /// The relay this endpoint uses now, which pairing codes advertise.
    pub relay_url: Arc<Mutex<Option<String>>>,
    pub relay_account: Option<Arc<Mutex<RelayAccountStatus>>>,
    /// Renews the paid-relay ticket.
    pub keeper: Option<tokio::task::JoinHandle<()>>,
    /// Unix time to rebuild the endpoint by, because its relay connection keeps
    /// the ticket it started with; 0 when none is due.
    pub restart_after_unix: Arc<AtomicU64>,
}

impl P2pGeneration {
    pub fn restart_due(&self, now_unix: u64) -> bool {
        let due = self.restart_after_unix.load(Ordering::Relaxed);
        due != 0 && due <= now_unix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(byte: u8) -> PairedDevice {
        PairedDevice {
            endpoint_id: EndpointKey([byte; 32]),
            label: format!("device {byte}"),
            preset: DevicePreset::Full,
            paired_at_millis: 1,
            last_seen_millis: None,
        }
    }

    #[test]
    fn devices_persist_and_revocation_takes_effect() {
        let root = tempfile::tempdir().unwrap();
        let registry = DeviceRegistry::load(root.path()).unwrap();
        registry.insert(device(1)).unwrap();
        registry.insert(device(2)).unwrap();
        assert!(registry.remove(EndpointKey([1; 32])).unwrap());
        assert!(!registry.contains(EndpointKey([1; 32])));

        let reloaded = DeviceRegistry::load(root.path()).unwrap();
        assert_eq!(reloaded.list(), vec![device(2)]);
        assert!(!reloaded.remove(EndpointKey([1; 32])).unwrap());
    }

    #[test]
    fn the_host_key_is_created_once() {
        let root = tempfile::tempdir().unwrap();
        let first = load_or_create_host_key(root.path()).unwrap();
        let second = load_or_create_host_key(root.path()).unwrap();
        assert_eq!(*first, *second);
    }

    #[test]
    fn an_invitation_expires_and_a_new_one_withdraws_the_old() {
        let mut slot = PairingSlot::default();
        let first = slot.open(1_000);
        assert!(slot.is_open(1_000));
        assert!(!slot.is_open(first.expires_millis));
        let second = slot.open(2_000);
        assert_ne!(first.id, second.id);
        assert!(matches!(slot.state(2_000), Some(PairingState::Open { .. })));
        slot.close(PairRejectReason::InvitationClosed);
        assert_eq!(slot.state(2_000), None);
    }
}
