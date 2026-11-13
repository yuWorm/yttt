//! The UniFFI surface the Swift and Kotlin apps call.
//!
//! The core owns a multi-threaded tokio runtime. Each async method spawns its
//! work there and awaits the `JoinHandle`, so the foreign executor that polls
//! it needs no tokio context. `drain`, `send_input` and `in_control` are
//! synchronous and return at once: the app calls `drain` every display frame.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::account::{Account, AccountError};
use crate::files::{FileEntry, OpenedFile, SaveOutcome, SessionEntry};
use crate::keys::TerminalKey;
use crate::overview::Overview;
use crate::session::{
    DrainStats, HostInfo, LinkPath, LinkState, MobileError, MobileSession, PairedHostRecord,
    RelayChoice, RelaySource, TerminalInfo,
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
    /// The relay account's session is no longer valid: forget it and ask the
    /// user to sign in again.
    #[error("the relay account's sign-in expired")]
    SignInExpired,
    /// A browser sign-in was cancelled.
    #[error("the sign-in was cancelled")]
    Cancelled,
}

impl From<AccountError> for MobileCoreError {
    fn from(error: AccountError) -> Self {
        match error {
            AccountError::SignInExpired => Self::SignInExpired,
            error => failed(error.to_string()),
        }
    }
}

impl From<MobileError> for MobileCoreError {
    fn from(error: MobileError) -> Self {
        match error {
            MobileError::HostTooOld => Self::HostTooOld,
            MobileError::SignInExpired => Self::SignInExpired,
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

fn account(bytes: Option<Vec<u8>>) -> Result<Option<Account>, MobileCoreError> {
    bytes
        .map(|bytes| Account::from_bytes(&bytes))
        .transpose()
        .map_err(Into::into)
}

/// A signed-in relay account. `account` is what the app keeps in the
/// Keychain or Keystore and passes back to pair and connect.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SignedIn {
    pub account: Vec<u8>,
    pub email: String,
    pub control_url: String,
}

impl From<Account> for SignedIn {
    fn from(account: Account) -> Self {
        Self {
            account: account.to_bytes(),
            email: account.email.clone(),
            control_url: account.session.control_url.clone(),
        }
    }
}

/// What the settings page shows about the account.
#[derive(Clone, Debug, uniffi::Record)]
pub struct AccountDetails {
    pub email: String,
    pub plan: String,
    pub devices: u32,
    pub max_devices: u32,
    pub suspended: bool,
}

/// A browser sign-in: open `authorize_url` in a browser, then `finish`
/// waits on a loopback port until the console sends the browser back.
#[derive(uniffi::Object)]
pub struct BrowserSignIn {
    authorize_url: String,
    login: Mutex<Option<yttt_client_core::control::BrowserLogin>>,
    cancelled: Arc<tokio::sync::Notify>,
    runtime: tokio::runtime::Handle,
}

#[uniffi::export]
impl BrowserSignIn {
    pub fn authorize_url(&self) -> String {
        self.authorize_url.clone()
    }

    /// Waits for the browser to come back, then for the account behind the
    /// session; `Cancelled` after `cancel`.
    pub async fn finish(&self) -> Result<SignedIn, MobileCoreError> {
        let login = self
            .login
            .lock()
            .take()
            .ok_or_else(|| failed("the sign-in has already finished"))?;
        let cancelled = self.cancelled.clone();
        self.runtime
            .spawn(async move {
                let session = tokio::select! {
                    session = login.finish() => session.map_err(AccountError::from)?,
                    () = cancelled.notified() => return Err(MobileCoreError::Cancelled),
                };
                Ok(Account::from_session(session).await?.into())
            })
            .await
            .map_err(|error| failed(error.to_string()))?
    }

    /// Stops waiting, as when the user closes the browser.
    pub fn cancel(&self) {
        self.cancelled.notify_one();
    }
}

fn secret(bytes: &[u8]) -> Result<[u8; 32], MobileCoreError> {
    bytes
        .try_into()
        .map_err(|_| failed("the device secret must be 32 bytes"))
}

/// Receives the home screen's data after each change, on a Rust thread;
/// switch to the main thread before touching the UI.
#[uniffi::export(with_foreign)]
pub trait OverviewListener: Send + Sync {
    fn on_overview(&self, overview: Overview);
}

/// Receives the link's state after each change, on a Rust thread.
#[uniffi::export(with_foreign)]
pub trait LinkListener: Send + Sync {
    fn on_link(&self, state: LinkState);
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

/// Logs to stderr, filtered by `YTTT_LOG` (as `RUST_LOG`), when it is set:
/// a diagnostic for a debug run, as `devicectl … --console` shows stderr.
fn log_from_env() {
    let Ok(filter) = std::env::var("YTTT_LOG") else {
        return;
    };
    let Ok(filter) = tracing_subscriber::EnvFilter::try_new(filter) else {
        return;
    };
    // A second core in the same process keeps the first one's subscriber.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}

#[uniffi::export]
impl MobileCore {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, MobileCoreError> {
        log_from_env();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("yttt-mobile-core")
            .build()
            .map_err(|error| failed(error.to_string()))?;
        Ok(Arc::new(Self { runtime }))
    }

    /// Signs in to a relay account with its email and password.
    pub async fn sign_in_password(
        &self,
        control_url: String,
        email: String,
        password: String,
    ) -> Result<SignedIn, MobileCoreError> {
        self.runtime
            .spawn(async move { Account::sign_in(&control_url, &email, &password).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Starts a sign-in in the browser; `label` names this phone on the
    /// console's page.
    pub async fn begin_browser_sign_in(
        &self,
        control_url: String,
        label: String,
    ) -> Result<Arc<BrowserSignIn>, MobileCoreError> {
        let login = self
            .runtime
            .spawn(async move { crate::account::begin_browser_sign_in(&control_url, &label).await })
            .await
            .map_err(|error| failed(error.to_string()))??;
        Ok(Arc::new(BrowserSignIn {
            authorize_url: login.authorize_url().to_owned(),
            login: Mutex::new(Some(login)),
            cancelled: Arc::new(tokio::sync::Notify::new()),
            runtime: self.runtime.handle().clone(),
        }))
    }

    /// The account's plan and use, for the settings page.
    pub async fn account_details(
        &self,
        account: Vec<u8>,
    ) -> Result<AccountDetails, MobileCoreError> {
        let account = Account::from_bytes(&account)?;
        let view = self
            .runtime
            .spawn(async move { account.session.account().await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(AccountError::from)?;
        Ok(AccountDetails {
            email: view.email,
            plan: view.plan,
            devices: view.devices,
            max_devices: view.max_devices,
            suspended: view.suspended,
        })
    }

    /// Removes this phone's key for the Host in `record` from the account,
    /// when the user forgets that Host.
    pub async fn release_host(
        &self,
        account: Vec<u8>,
        record: Vec<u8>,
        device_secret: Vec<u8>,
    ) -> Result<(), MobileCoreError> {
        let account = Account::from_bytes(&account)?;
        let record = PairedHostRecord::from_bytes(&record)?;
        let device_secret = secret(&device_secret)?;
        self.runtime
            .spawn(async move { account.release(&device_secret, &record.host.host).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// Pairs with the Host in `code` and returns the record to store.
    /// `relay_only` keeps the device off direct paths (a diagnostic setting);
    /// `account`, a signed-in relay account, pairs through the Host's relay
    /// node with a ticket.
    pub async fn pair(
        &self,
        code: String,
        device_label: String,
        device_secret: Vec<u8>,
        relay_only: bool,
        account: Option<Vec<u8>>,
        listener: Arc<dyn PairingListener>,
    ) -> Result<Vec<u8>, MobileCoreError> {
        let device_secret = secret(&device_secret)?;
        let account = self::account(account)?;
        let work = self.runtime.spawn(async move {
            crate::session::pair(
                &code,
                &device_label,
                device_secret,
                relay_choice(relay_only),
                account.as_ref(),
                move |short_code| listener.on_short_code(short_code),
            )
            .await
        });
        let record = work.await.map_err(|error| failed(error.to_string()))??;
        Ok(record.to_bytes())
    }

    /// Which Host a stored record names, and what it is called.
    pub fn host_info(&self, record: Vec<u8>) -> Result<HostInfo, MobileCoreError> {
        Ok(PairedHostRecord::from_bytes(&record)?.info())
    }

    /// Connects to a paired Host; `account` as for `pair`, `device_label`
    /// naming this phone on the account's console.
    pub async fn connect(
        &self,
        record: Vec<u8>,
        device_secret: Vec<u8>,
        relay_only: bool,
        account: Option<Vec<u8>>,
        device_label: String,
    ) -> Result<Arc<HostSession>, MobileCoreError> {
        let record = PairedHostRecord::from_bytes(&record)?;
        let device_secret = secret(&device_secret)?;
        let account = self::account(account)?;
        let work = self.runtime.spawn(async move {
            MobileSession::connect(
                &record,
                device_secret,
                relay_choice(relay_only),
                account.as_ref(),
                &device_label,
            )
            .await
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

    /// Where the relay this session uses came from.
    pub fn relay_source(&self) -> RelaySource {
        self.session.relay_source()
    }

    /// Whether the session reaches the Host now.
    pub fn link_state(&self) -> LinkState {
        self.session.link_state()
    }

    /// Calls `listener` with the link state now and after each change;
    /// replaces an earlier listener and ends on `disconnect`.
    pub fn watch_link(&self, listener: Arc<dyn LinkListener>) {
        let _runtime = self.runtime.enter();
        self.session
            .watch_link(move |state| listener.on_link(state));
    }

    /// Reconnects at once instead of after the backoff.
    pub fn retry_now(&self) {
        self.session.retry_now();
    }

    /// Opens the workspace the desktop shows, so the overview has its
    /// projects and tabs; returns its name, or `None` when there is none.
    pub async fn open_active_workspace(&self) -> Result<Option<String>, MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.open_active_workspace().await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map(|summary| summary.map(|summary| summary.name))
            .map_err(Into::into)
    }

    /// One directory of a project, `""` for its root.
    pub async fn list_directory(
        &self,
        project_id: String,
        directory: String,
    ) -> Result<Vec<FileEntry>, MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.list_directory(&project_id, &directory).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// Reads a file to edit; `save_file` then checks it is unchanged.
    pub async fn open_file(
        &self,
        project_id: String,
        path: String,
    ) -> Result<OpenedFile, MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.open_file(&project_id, &path).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// Saves an opened file; `Conflict` when it changed on the Host, unless
    /// `force`.
    pub async fn save_file(
        &self,
        project_id: String,
        path: String,
        text: String,
        force: bool,
    ) -> Result<SaveOutcome, MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.save_file(&project_id, &path, text, force).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// The Agent sessions the Host has for a project.
    pub async fn agent_sessions(
        &self,
        project_id: String,
    ) -> Result<Vec<SessionEntry>, MobileCoreError> {
        let session = self.session.clone();
        self.runtime
            .spawn(async move { session.agent_sessions(&project_id).await })
            .await
            .map_err(|error| failed(error.to_string()))?
            .map_err(Into::into)
    }

    /// The home screen's data as it is now.
    pub fn overview(&self) -> Overview {
        self.session.overview()
    }

    /// Calls `listener` with the overview now and after each change,
    /// debounced; replaces an earlier listener and ends on `disconnect`.
    pub fn watch_overview(&self, listener: Arc<dyn OverviewListener>) {
        let _runtime = self.runtime.enter();
        self.session
            .watch_overview(move |overview| listener.on_overview(overview));
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

    /// A key-bar key, encoded for the program's cursor-key mode.
    pub fn send_key(&self, session_id: String, key: TerminalKey) -> Result<(), MobileCoreError> {
        Ok(self.session.send_key(&session_id, key)?)
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
