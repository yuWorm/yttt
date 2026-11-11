//! The UniFFI surface the Swift and Kotlin apps call.
//!
//! The core owns a multi-threaded tokio runtime. Each async method spawns its
//! work there and awaits the `JoinHandle`, so the foreign executor that polls
//! it needs no tokio context. `drain`, `send_input` and `in_control` are
//! synchronous and return at once: the app calls `drain` every display frame.

use std::sync::Arc;

use crate::session::{
    DrainStats, MobileError, MobileSession, PairedHostRecord, RelayChoice, TerminalInfo,
};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileCoreError {
    // Not `message`: Kotlin exceptions already have one.
    #[error("{reason}")]
    Failed { reason: String },
}

impl From<MobileError> for MobileCoreError {
    fn from(error: MobileError) -> Self {
        Self::Failed {
            reason: error.to_string(),
        }
    }
}

fn failed(message: impl Into<String>) -> MobileCoreError {
    MobileCoreError::Failed {
        reason: message.into(),
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
    pub async fn pair(
        &self,
        code: String,
        device_label: String,
        device_secret: Vec<u8>,
        listener: Arc<dyn PairingListener>,
    ) -> Result<Vec<u8>, MobileCoreError> {
        let device_secret = secret(&device_secret)?;
        let work = self.runtime.spawn(async move {
            crate::session::pair(
                &code,
                &device_label,
                device_secret,
                RelayChoice::FromHost,
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
    ) -> Result<Arc<HostSession>, MobileCoreError> {
        let record = PairedHostRecord::from_bytes(&record)?;
        let device_secret = secret(&device_secret)?;
        let work = self.runtime.spawn(async move {
            MobileSession::connect(&record, device_secret, RelayChoice::FromHost).await
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

    pub async fn take_control(&self) -> Result<(), MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.take_control().await })
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
