//! Appends Host connection changes to a bounded log, so a disconnect can be
//! traced afterwards: when it happened, why, and how long recovery took.

use std::{fs::OpenOptions, io::Write as _, path::Path, sync::Arc};

use yttt_client_core::{ClientCore, ConnectionState};

const LOG_FILE: &str = "client.log";
/// Past this size the log moves to `client.log.1`, replacing an older one.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Logs the connection changes of `client` to `logs/client.log` until the
/// Client stops.
pub(crate) fn follow(
    client: Arc<ClientCore>,
    runtime: &tokio::runtime::Handle,
    logs: &Path,
    label: String,
) {
    let path = logs.join(LOG_FILE);
    let mut state = client.subscribe_state();
    runtime.spawn(async move {
        while state.changed().await.is_ok() {
            let current = state.borrow_and_update().clone();
            if let Some(line) = describe(&client, &current) {
                let path = path.clone();
                let line = format!(
                    "{} [{label}] {line}\n",
                    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                );
                let _ = tokio::task::spawn_blocking(move || append(&path, &line)).await;
            }
        }
    });
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

fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        std::fs::rename(path, path.with_extension("log.1"))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_rotates_once_it_outgrows_its_bound() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(LOG_FILE);
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        append(&path, "next\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next\n");
        assert!(temp.path().join("client.log.1").exists());
    }
}
