use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientPipelineDiagnosticsSnapshot {
    pub ipc_payload_bytes: u64,
    pub ipc_read_and_decode: ClientLatencyDiagnosticsSnapshot,
    pub terminal_merge: ClientLatencyDiagnosticsSnapshot,
    pub checkpoint_resyncs: u64,
    pub connection_attempts: u64,
    pub reconnects: u64,
    pub heartbeat_round_trip: ClientLatencyDiagnosticsSnapshot,
}

#[derive(Default)]
pub(crate) struct ClientPipelineDiagnostics {
    ipc_payload_bytes: AtomicU64,
    ipc_read_and_decode: ClientLatencyDiagnostics,
    terminal_merge: ClientLatencyDiagnostics,
    checkpoint_resyncs: AtomicU64,
    connection_attempts: AtomicU64,
    reconnects: AtomicU64,
    heartbeat_round_trip: ClientLatencyDiagnostics,
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

    pub(crate) fn record_checkpoint_resync(&self) {
        self.checkpoint_resyncs.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_connection_attempt(&self) {
        self.connection_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_heartbeat(&self, duration: Duration) {
        self.heartbeat_round_trip.record(duration);
    }

    pub(crate) fn snapshot(&self) -> ClientPipelineDiagnosticsSnapshot {
        ClientPipelineDiagnosticsSnapshot {
            ipc_payload_bytes: self.ipc_payload_bytes.load(Ordering::Acquire),
            ipc_read_and_decode: self.ipc_read_and_decode.snapshot(),
            terminal_merge: self.terminal_merge.snapshot(),
            checkpoint_resyncs: self.checkpoint_resyncs.load(Ordering::Acquire),
            connection_attempts: self.connection_attempts.load(Ordering::Acquire),
            reconnects: self.reconnects.load(Ordering::Acquire),
            heartbeat_round_trip: self.heartbeat_round_trip.snapshot(),
        }
    }
}
