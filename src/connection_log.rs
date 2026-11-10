//! Logs Host connection changes to `client.log`, so a disconnect can be
//! traced afterwards: when it happened, why, and how long recovery took.
//! The file itself belongs to the process's log subscriber ([`install`]).

use std::{path::Path, sync::Arc};

use yttt_client_core::{ClientCore, ConnectionState};

const LOG_FILE: &str = "client.log";
/// Past this size the log moves to `client.log.1`, replacing an older one.
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

/// Sends this process's log lines to `logs/client.log`. Its detail starts
/// normal and follows the settings once they are read.
/// `process` names this process in each line, as `desktop` or
/// `remote-window`, since both write the same file.
pub(crate) fn install(logs: &Path, process: &'static str) {
    let _ = std::fs::create_dir_all(logs);
    yttt_host::logging::install(
        yttt_host::logging::RotatingFile::new(logs.join(LOG_FILE), MAX_LOG_BYTES),
        process,
        false,
    );
}

/// Logs the connection changes of `client` until the Client stops.
pub(crate) fn follow(client: Arc<ClientCore>, runtime: &tokio::runtime::Handle, label: String) {
    let mut state = client.subscribe_state();
    runtime.spawn(async move {
        while state.changed().await.is_ok() {
            let current = state.borrow_and_update().clone();
            if let Some(line) = describe(&client, &current) {
                note(&label, &line);
            }
        }
    });
}

/// Logs one line about the connection labelled `label`.
pub(crate) fn note(label: &str, line: &str) {
    tracing::info!(target: "yttt::connection", "[{label}] {line}");
}

fn describe(client: &ClientCore, state: &ConnectionState) -> Option<String> {
    match state {
        ConnectionState::Reconnecting {
            attempt: 1,
            message,
        } => Some(format!("disconnected: {message}")),
        ConnectionState::Reconnecting { .. } | ConnectionState::Connecting => None,
        ConnectionState::Ready { host_epoch, .. } => {
            let recovered = client
                .diagnostics()
                .recent_disconnects
                .last()
                .and_then(|record| record.recovered_after)?;
            Some(format!(
                "reconnected to Host epoch {host_epoch} after {:.1} s",
                recovered.as_secs_f64()
            ))
        }
        ConnectionState::HostLost { message } => Some(format!("Host lost: {message}")),
        ConnectionState::Disconnected => Some("disconnected".to_string()),
    }
}
