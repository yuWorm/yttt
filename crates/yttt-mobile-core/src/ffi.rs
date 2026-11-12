//! The UniFFI surface the Swift and Kotlin apps call.
//!
//! The core owns a multi-threaded tokio runtime. Each async method spawns its
//! work there and awaits the `JoinHandle`, so the foreign executor that polls
//! it needs no tokio context. `drain`, `send_input` and `in_control` are
//! synchronous and return at once: the app calls `drain` every display frame.

use std::sync::Arc;

use crate::session::{
    DrainStats, LinkPath, MobileError, MobileSession, PairedHostRecord, RelayChoice, TerminalInfo,
};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileCoreError {
    // Not `message`: Kotlin exceptions already have one.
    #[error("{reason}")]
    Failed { reason: String },
    /// The Host cannot hand over a single terminal: hide taking control and
    /// suggest updating yttt on the computer.
    #[error("the Host needs a newer yttt")]
    HostTooOld,
}

impl From<MobileError> for MobileCoreError {
    fn from(error: MobileError) -> Self {
        match error {
            MobileError::HostTooOld => Self::HostTooOld,
            error => Self::Failed {
                reason: error.to_string(),
            },
        }
    }
}

fn failed(message: impl Into<String>) -> MobileCoreError {
    MobileCoreError::Failed {
        reason: message.into(),
    }
}

fn relay_choice(relay_only: bool) -> RelayChoice {
    if relay_only {
        RelayChoice::OnlyFromHost
    } else {
        RelayChoice::FromHost
    }
}

fn secret(bytes: &[u8]) -> Result<[u8; 32], MobileCoreError> {
    bytes
        .try_into()
        .map_err(|_| failed("the device secret must be 32 bytes"))
}

/// Receives the six-digit pairing code to show while the Host user decides.
#[uniffi::export(with_foreign)]
pub trait PairingListener: Send + Sync {
    fn on_short_code(&self, short_code: u32);
}

#[derive(uniffi::Object)]
pub struct MobileCore {
    runtime: tokio::runtime::Runtime,
}

#[uniffi::export]
impl MobileCore {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, MobileCoreError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("yttt-mobile-core")
            .build()
            .map_err(|error| failed(error.to_string()))?;
        Ok(Arc::new(Self { runtime }))
    }

    /// Pairs with the Host in `code` and returns the record to store.
    /// `relay_only` keeps the device off direct paths (a diagnostic setting).
    pub async fn pair(
        &self,
        code: String,
        device_label: String,
        device_secret: Vec<u8>,
        relay_only: bool,
        listener: Arc<dyn PairingListener>,
    ) -> Result<Vec<u8>, MobileCoreError> {
        let device_secret = secret(&device_secret)?;
        let work = self.runtime.spawn(async move {
            crate::session::pair(
                &code,
                &device_label,
                device_secret,
                relay_choice(relay_only),
                move |short_code| listener.on_short_code(short_code),
            )
            .await
        });
        let record = work.await.map_err(|error| failed(error.to_string()))??;
        Ok(record.to_bytes())
    }

    /// The label of the Host a stored record names.
    pub fn host_label(&self, record: Vec<u8>) -> Result<String, MobileCoreError> {
        Ok(PairedHostRecord::from_bytes(&record)?.host.host_label)
    }

    pub async fn connect(
        &self,
        record: Vec<u8>,
        device_secret: Vec<u8>,
        relay_only: bool,
    ) -> Result<Arc<HostSession>, MobileCoreError> {
        let record = PairedHostRecord::from_bytes(&record)?;
        let device_secret = secret(&device_secret)?;
        let work = self.runtime.spawn(async move {
            MobileSession::connect(&record, device_secret, relay_choice(relay_only)).await
        });
        let session = work.await.map_err(|error| failed(error.to_string()))??;
        Ok(Arc::new(HostSession {
            session: Arc::new(session),
            runtime: self.runtime.handle().clone(),
        }))
    }
}

/// A connected Host.
#[derive(uniffi::Object)]
pub struct HostSession {
    session: Arc<MobileSession>,
    runtime: tokio::runtime::Handle,
}

#[uniffi::export]
impl HostSession {
    pub fn is_ready(&self) -> bool {
        self.session.is_ready()
    }

    pub async fn terminals(&self) -> Result<Vec<TerminalInfo>, MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.terminals().await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    pub async fn attach(&self, session_id: String) -> Result<(), MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.attach(&session_id).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    pub async fn detach(&self, session_id: String) -> Result<(), MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.detach(&session_id).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// Repaint bytes since the last call; `None` when nothing changed.
    pub fn drain(&self, session_id: String) -> Option<Vec<u8>> {
        self.session.drain(&session_id)
    }

    pub fn drain_stats(&self) -> DrainStats {
        self.session.drain_stats()
    }

    /// Call when the device's network changes (path monitor, connectivity
    /// callback). Returns at once; the check runs in the background.
    pub fn network_changed(&self) {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.network_changed().await });
    }

    /// Whether data goes straight to the Host or through the relay, for the
    /// connection status the app shows.
    pub async fn path(&self) -> LinkPath {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.path().await })
            .await
            .unwrap_or(LinkPath::Unknown)
    }

    /// The next `drain` repaints the whole terminal; call it after the view
    /// changed its grid size and cleared itself.
    pub fn repaint(&self, session_id: String) {
        self.session.repaint(&session_id);
    }

    /// Takes this one terminal at the device's size; input is accepted once
    /// it returns.
    pub async fn take_control(
        &self,
        session_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.take_control(&session_id, cols, rows).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// Gives terminals back to the computer; an empty list gives back every
    /// one this device took. Call it whenever the app leaves the foreground.
    pub async fn give_back(&self, session_ids: Vec<String>) -> Result<(), MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.give_back(session_ids).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    pub fn in_control(&self, session_id: String) -> bool {
        self.session.in_control(&session_id)
    }

    pub fn send_input(&self, session_id: String, bytes: Vec<u8>) -> Result<(), MobileCoreError> {
        Ok(self.session.send_input(&session_id, bytes)?)
    }

    pub async fn resize(
        &self,
        session_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.resize(&session_id, cols, rows).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// Not `close`: Kotlin's `AutoCloseable.close` would clash.
    pub async fn disconnect(&self) {
        let session = self.session.clone();
        let _ = self
            .runtime
            .spawn(async move { session.close().await })
            .await;
    }
}
