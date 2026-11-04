use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;

use yttt_transport::WireReceiveDiagnostics;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientLatencyDiagnosticsSnapshot {
    pub samples: u64,
    pub total_nanos: u64,
    pub max_nanos: u64,
}

#[derive(Default)]
struct ClientLatencyDiagnostics {
    samples: AtomicU64,
    total_nanos: AtomicU64,
    max_nanos: AtomicU64,
}

impl ClientLatencyDiagnostics {
    fn record(&self, duration: Duration) {
        let nanos = duration.as_nanos().min(u64::MAX as u128) as u64;
        self.samples.fetch_add(1, Ordering::Relaxed);
        self.total_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.max_nanos.fetch_max(nanos, Ordering::Relaxed);
    }

    fn snapshot(&self) -> ClientLatencyDiagnosticsSnapshot {
        ClientLatencyDiagnosticsSnapshot {
            samples: self.samples.load(Ordering::Acquire),
            total_nanos: self.total_nanos.load(Ordering::Acquire),
            max_nanos: self.max_nanos.load(Ordering::Acquire),
        }
    }
}

/// How many recent disconnects diagnostics keep.
const RECENT_DISCONNECTS: usize = 16;

/// One time the Client lost its Host session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisconnectRecord {
    /// Wall-clock time of the disconnect, in Unix milliseconds.
    pub at_unix_millis: u64,
    pub reason: String,
    /// How long until the session was ready again; `None` while it is not.
    pub recovered_after: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientPipelineDiagnosticsSnapshot {
    pub ipc_payload_bytes: u64,
    pub ipc_read_and_decode: ClientLatencyDiagnosticsSnapshot,
    pub terminal_merge: ClientLatencyDiagnosticsSnapshot,
    pub connection_attempts: u64,
    pub reconnects: u64,
    pub heartbeat_round_trip: ClientLatencyDiagnosticsSnapshot,
    /// The most recent disconnects, oldest first.
    pub recent_disconnects: Vec<DisconnectRecord>,
}

#[derive(Default)]
pub(crate) struct ClientPipelineDiagnostics {
    ipc_payload_bytes: AtomicU64,
    ipc_read_and_decode: ClientLatencyDiagnostics,
    terminal_merge: ClientLatencyDiagnostics,
    connection_attempts: AtomicU64,
    reconnects: AtomicU64,
    heartbeat_round_trip: ClientLatencyDiagnostics,
    disconnects: Mutex<VecDeque<(DisconnectRecord, std::time::Instant)>>,
}

impl ClientPipelineDiagnostics {
    pub(crate) fn record_ipc_read(&self, observation: WireReceiveDiagnostics) {
        self.ipc_payload_bytes
            .fetch_add(observation.payload_bytes as u64, Ordering::Relaxed);
        self.ipc_read_and_decode.record(
            observation
                .payload_read_and_check
                .saturating_add(observation.message_decode),
        );
    }

    pub(crate) fn record_terminal_merge(&self, duration: Duration) {
        self.terminal_merge.record(duration);
    }

    pub(crate) fn record_connection_attempt(&self) {
        self.connection_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_disconnect(&self, reason: &str) -> DisconnectRecord {
        let record = DisconnectRecord {
            at_unix_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_millis() as u64),
            reason: reason.to_string(),
            recovered_after: None,
        };
        let mut disconnects = self.disconnects.lock();
        if disconnects.len() == RECENT_DISCONNECTS {
            disconnects.pop_front();
        }
        disconnects.push_back((record.clone(), std::time::Instant::now()));
        record
    }

    /// Marks the latest disconnect recovered; returns it with its duration.
    pub(crate) fn record_recovered(&self) -> Option<DisconnectRecord> {
        let mut disconnects = self.disconnects.lock();
        let (record, since) = disconnects.back_mut()?;
        if record.recovered_after.is_some() {
            return None;
        }
        record.recovered_after = Some(since.elapsed());
        Some(record.clone())
    }

    pub(crate) fn record_heartbeat(&self, duration: Duration) {
        self.heartbeat_round_trip.record(duration);
    }

    pub(crate) fn snapshot(&self) -> ClientPipelineDiagnosticsSnapshot {
        ClientPipelineDiagnosticsSnapshot {
            ipc_payload_bytes: self.ipc_payload_bytes.load(Ordering::Acquire),
            ipc_read_and_decode: self.ipc_read_and_decode.snapshot(),
            terminal_merge: self.terminal_merge.snapshot(),
            connection_attempts: self.connection_attempts.load(Ordering::Acquire),
            reconnects: self.reconnects.load(Ordering::Acquire),
            heartbeat_round_trip: self.heartbeat_round_trip.snapshot(),
            recent_disconnects: self
                .disconnects
                .lock()
                .iter()
                .map(|(record, _)| record.clone())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnects_record_their_reason_and_recovery() {
        let diagnostics = ClientPipelineDiagnostics::default();
        assert!(diagnostics.record_recovered().is_none());
        for index in 0..RECENT_DISCONNECTS + 2 {
            diagnostics.record_disconnect(&format!("drop {index}"));
        }
        let recovered = diagnostics.record_recovered().expect("latest disconnect");
        assert!(recovered.recovered_after.is_some());
        assert!(diagnostics.record_recovered().is_none(), "recovered once");
        let recent = diagnostics.snapshot().recent_disconnects;
        assert_eq!(recent.len(), RECENT_DISCONNECTS);
        assert_eq!(recent.first().unwrap().reason, "drop 2");
        assert_eq!(
            recent.last().unwrap().reason,
            format!("drop {}", RECENT_DISCONNECTS + 1)
        );
        assert!(recent.last().unwrap().recovered_after.is_some());
    }
}
