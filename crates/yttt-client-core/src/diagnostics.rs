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
/// How many recent heartbeat round trips the shown latency is the median of.
const RECENT_ROUND_TRIPS: usize = 5;

/// How the session's link behaves now, for the window to show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkHealth {
    /// Median of the recent heartbeat round trips.
    pub round_trip: Option<Duration>,
    /// A heartbeat has waited for its reply longer than the link's
    /// `unstable_after`; the session is still up.
    pub unstable: bool,
    /// How long the heartbeat still unanswered has waited; `None` when every
    /// heartbeat has its reply.
    pub waiting_for: Option<Duration>,
}

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
    round_trips: Mutex<RoundTrips>,
}

#[derive(Default)]
struct RoundTrips {
    recent: VecDeque<Duration>,
    /// When the heartbeat awaiting its reply was sent.
    awaiting_since: Option<std::time::Instant>,
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
        let mut round_trips = self.round_trips.lock();
        round_trips.awaiting_since = None;
        if round_trips.recent.len() == RECENT_ROUND_TRIPS {
            round_trips.recent.pop_front();
        }
        round_trips.recent.push_back(duration);
    }

    pub(crate) fn record_heartbeat_sent(&self, at: std::time::Instant) {
        self.round_trips.lock().awaiting_since = Some(at);
    }

    /// Forgets the link of a session that ended.
    pub(crate) fn clear_link_health(&self) {
        *self.round_trips.lock() = RoundTrips::default();
    }

    pub(crate) fn link_health(
        &self,
        now: std::time::Instant,
        unstable_after: Duration,
    ) -> LinkHealth {
        let round_trips = self.round_trips.lock();
        let mut recent = round_trips.recent.iter().copied().collect::<Vec<_>>();
        recent.sort_unstable();
        let waiting_for = round_trips
            .awaiting_since
            .map(|sent| now.saturating_duration_since(sent));
        LinkHealth {
            round_trip: recent.get(recent.len() / 2).copied(),
            unstable: waiting_for.is_some_and(|waited| waited >= unstable_after),
            waiting_for,
        }
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

    #[test]
    fn link_health_reports_the_median_round_trip_and_a_late_reply() {
        let diagnostics = ClientPipelineDiagnostics::default();
        let start = std::time::Instant::now();
        let unstable_after = Duration::from_secs(6);
        assert_eq!(
            diagnostics.link_health(start, unstable_after),
            LinkHealth::default()
        );
        for millis in [40, 900, 30, 50, 45, 35] {
            diagnostics.record_heartbeat_sent(start);
            diagnostics.record_heartbeat(Duration::from_millis(millis));
        }
        // The last five, so one slow reply does not set the figure.
        let health = diagnostics.link_health(start, unstable_after);
        assert_eq!(health.round_trip, Some(Duration::from_millis(45)));
        assert!(!health.unstable);

        diagnostics.record_heartbeat_sent(start);
        let waiting = diagnostics.link_health(start + Duration::from_secs(3), unstable_after);
        assert!(!waiting.unstable);
        let late = diagnostics.link_health(start + Duration::from_secs(7), unstable_after);
        assert!(late.unstable);
        assert_eq!(late.waiting_for, Some(Duration::from_secs(7)));
        assert_eq!(late.round_trip, Some(Duration::from_millis(45)));

        diagnostics.clear_link_health();
        assert_eq!(
            diagnostics.link_health(start + Duration::from_secs(7), unstable_after),
            LinkHealth::default()
        );
    }
}
