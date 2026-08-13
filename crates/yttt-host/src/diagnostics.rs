use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub const DIAGNOSTICS_SCHEMA_VERSION: u32 = 2;
pub const DEFAULT_DIAGNOSTICS_LOG_BYTES: u64 = 4 * 1024 * 1024;
const LATENCY_BUCKETS: usize = 64;

pub trait DiagnosticsClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

#[derive(Default)]
pub struct SystemDiagnosticsClock;

impl DiagnosticsClock for SystemDiagnosticsClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyDiagnosticsSnapshot {
    pub samples: u64,
    pub total_nanos: u64,
    pub max_nanos: u64,
    pub p50_nanos: Option<u64>,
    pub p95_nanos: Option<u64>,
    pub p99_nanos: Option<u64>,
}

pub(crate) struct LatencyDiagnostics {
    samples: AtomicU64,
    total_nanos: AtomicU64,
    max_nanos: AtomicU64,
    buckets: [AtomicU64; LATENCY_BUCKETS],
}

impl Default for LatencyDiagnostics {
    fn default() -> Self {
        Self {
            samples: AtomicU64::new(0),
            total_nanos: AtomicU64::new(0),
            max_nanos: AtomicU64::new(0),
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl LatencyDiagnostics {
    pub(crate) fn record(&self, duration: Duration) {
        let nanos = duration.as_nanos().min(u64::MAX as u128) as u64;
        self.samples.fetch_add(1, Ordering::Relaxed);
        self.total_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.max_nanos.fetch_max(nanos, Ordering::Relaxed);
        self.buckets[bucket_for_nanos(nanos)].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn reset(&self) {
        self.samples.store(0, Ordering::Release);
        self.total_nanos.store(0, Ordering::Release);
        self.max_nanos.store(0, Ordering::Release);
        for bucket in &self.buckets {
            bucket.store(0, Ordering::Release);
        }
    }

    pub(crate) fn snapshot(&self) -> LatencyDiagnosticsSnapshot {
        let samples = self.samples.load(Ordering::Acquire);
        LatencyDiagnosticsSnapshot {
            samples,
            total_nanos: self.total_nanos.load(Ordering::Acquire),
            max_nanos: self.max_nanos.load(Ordering::Acquire),
            p50_nanos: percentile_nanos(&self.buckets, samples, 50),
            p95_nanos: percentile_nanos(&self.buckets, samples, 95),
            p99_nanos: percentile_nanos(&self.buckets, samples, 99),
        }
    }
}

fn bucket_for_nanos(nanos: u64) -> usize {
    if nanos <= 1 {
        0
    } else {
        (u64::BITS - (nanos - 1).leading_zeros()) as usize
    }
    .min(LATENCY_BUCKETS - 1)
}

fn percentile_nanos(
    buckets: &[AtomicU64; LATENCY_BUCKETS],
    samples: u64,
    percentile: u64,
) -> Option<u64> {
    if samples == 0 {
        return None;
    }
    let rank = samples.saturating_mul(percentile).div_ceil(100);
    let mut cumulative = 0_u64;
    for (index, bucket) in buckets.iter().enumerate() {
        cumulative = cumulative.saturating_add(bucket.load(Ordering::Acquire));
        if cumulative >= rank {
            return Some(if index == 0 { 1 } else { 1_u64 << index });
        }
    }
    Some(u64::MAX)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueDiagnosticsSnapshot {
    pub name: String,
    pub current: usize,
    pub high_water: usize,
    pub capacity: usize,
    pub dropped: u64,
    pub resyncs: u64,
    pub service: LatencyDiagnosticsSnapshot,
}

pub(crate) struct QueueDiagnostics {
    name: &'static str,
    capacity: usize,
    current: AtomicUsize,
    high_water: AtomicUsize,
    dropped: AtomicU64,
    resyncs: AtomicU64,
    service: LatencyDiagnostics,
}

impl QueueDiagnostics {
    pub(crate) fn new(name: &'static str, capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            name,
            capacity,
            current: AtomicUsize::new(0),
            high_water: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            resyncs: AtomicU64::new(0),
            service: LatencyDiagnostics::default(),
        })
    }

    pub(crate) fn observe(&self, current: usize) {
        let current = current.min(self.capacity);
        self.current.store(current, Ordering::Release);
        self.high_water.fetch_max(current, Ordering::AcqRel);
    }

    pub(crate) fn dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn resync(&self) {
        self.resyncs.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_service(&self, duration: Duration) {
        self.service.record(duration);
    }

    pub(crate) fn reset(&self) {
        self.current.store(0, Ordering::Release);
        self.high_water.store(0, Ordering::Release);
        self.dropped.store(0, Ordering::Release);
        self.resyncs.store(0, Ordering::Release);
        self.service.reset();
    }

    pub(crate) fn snapshot(&self) -> QueueDiagnosticsSnapshot {
        QueueDiagnosticsSnapshot {
            name: self.name.to_string(),
            current: self.current.load(Ordering::Acquire),
            high_water: self.high_water.load(Ordering::Acquire),
            capacity: self.capacity,
            dropped: self.dropped.load(Ordering::Acquire),
            resyncs: self.resyncs.load(Ordering::Acquire),
            service: self.service.snapshot(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPipelineDiagnosticsSnapshot {
    pub session_id: String,
    pub subscribers: usize,
    pub bytes_parsed: u64,
    pub semantic_encode_count: u64,
    pub shared_ipc_encode_count: u64,
    pub skipped_unsubscribed_captures: u64,
    pub parser: LatencyDiagnosticsSnapshot,
    pub semantic_encode: LatencyDiagnosticsSnapshot,
    pub input_to_pty: LatencyDiagnosticsSnapshot,
    pub queues: Vec<QueueDiagnosticsSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostDiagnosticsSnapshot {
    pub schema_version: u32,
    pub captured_at_millis: u64,
    pub host_epoch: u64,
    pub sessions: usize,
    pub clients: usize,
    pub attachments: usize,
    pub rss_bytes: Option<u64>,
    pub queues: Vec<QueueDiagnosticsSnapshot>,
    pub thread_count: Option<usize>,
    pub idle_cpu_percent: Option<f32>,
    pub terminals: Vec<TerminalPipelineDiagnosticsSnapshot>,
}

pub trait DiagnosticsSink: Send + Sync {
    fn record(&self, snapshot: &HostDiagnosticsSnapshot) -> io::Result<()>;
}

#[derive(Default)]
pub struct MemoryDiagnosticsSink {
    snapshots: Mutex<Vec<HostDiagnosticsSnapshot>>,
}

impl MemoryDiagnosticsSink {
    pub fn snapshots(&self) -> Vec<HostDiagnosticsSnapshot> {
        self.snapshots.lock().clone()
    }
}

impl DiagnosticsSink for MemoryDiagnosticsSink {
    fn record(&self, snapshot: &HostDiagnosticsSnapshot) -> io::Result<()> {
        self.snapshots.lock().push(snapshot.clone());
        Ok(())
    }
}

pub struct RotatingJsonlDiagnosticsSink {
    path: PathBuf,
    maximum_bytes: u64,
    lock: Mutex<()>,
}

impl RotatingJsonlDiagnosticsSink {
    pub fn new(path: PathBuf, maximum_bytes: u64) -> Self {
        Self {
            path,
            maximum_bytes: maximum_bytes.max(1),
            lock: Mutex::new(()),
        }
    }

    fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.file_name().map_or_else(
            || "host-diagnostics.jsonl".into(),
            |name| name.to_os_string(),
        );
        name.push(".1");
        self.path.with_file_name(name)
    }

    fn rotate_if_needed(&self, additional_bytes: u64) -> io::Result<()> {
        let current = fs::metadata(&self.path).map_or(0, |metadata| metadata.len());
        if current.saturating_add(additional_bytes) <= self.maximum_bytes {
            return Ok(());
        }
        let rotated = self.rotated_path();
        match fs::remove_file(&rotated) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match fs::rename(&self.path, rotated) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl DiagnosticsSink for RotatingJsonlDiagnosticsSink {
    fn record(&self, snapshot: &HostDiagnosticsSnapshot) -> io::Result<()> {
        let _guard = self.lock.lock();
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut encoded = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
        encoded.push(b'\n');
        self.rotate_if_needed(encoded.len() as u64)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&encoded)?;
        file.flush()
    }
}

#[cfg(target_os = "macos")]
fn current_thread_count(_process: &sysinfo::Process) -> Option<usize> {
    use libproc::{proc_pid::pidinfo, task_info::TaskInfo};

    pidinfo::<TaskInfo>(std::process::id() as i32, 0)
        .ok()
        .and_then(|info| usize::try_from(info.pti_threadnum).ok())
        .filter(|count| *count > 0)
}

#[cfg(not(target_os = "macos"))]
fn current_thread_count(process: &sysinfo::Process) -> Option<usize> {
    process.tasks().map(|tasks| tasks.len())
}

pub struct ProcessDiagnosticsSampler {
    system: System,
    pid: Pid,
}

impl Default for ProcessDiagnosticsSampler {
    fn default() -> Self {
        Self {
            system: System::new(),
            pid: Pid::from_u32(std::process::id()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProcessDiagnosticsSample {
    pub rss_bytes: Option<u64>,
    pub thread_count: Option<usize>,
    pub cpu_percent: Option<f32>,
}

impl ProcessDiagnosticsSampler {
    pub fn sample(&mut self) -> ProcessDiagnosticsSample {
        let pids = [self.pid];
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        self.system
            .process(self.pid)
            .map(|process| ProcessDiagnosticsSample {
                rss_bytes: Some(process.memory()),
                thread_count: current_thread_count(process),
                cpu_percent: Some(process.cpu_usage()),
            })
            .unwrap_or_default()
    }
}

pub fn diagnostics_log_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join("host-diagnostics.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[derive(Default)]
    struct FakeClock(AtomicU64);

    impl DiagnosticsClock for FakeClock {
        fn now_millis(&self) -> u64 {
            self.0.load(Ordering::Acquire)
        }
    }

    #[test]
    fn memory_sink_and_fake_clock_are_deterministic() {
        let clock = FakeClock::default();
        clock.0.store(42, Ordering::Release);
        let sink = MemoryDiagnosticsSink::default();
        let snapshot = HostDiagnosticsSnapshot {
            schema_version: DIAGNOSTICS_SCHEMA_VERSION,
            captured_at_millis: clock.now_millis(),
            ..HostDiagnosticsSnapshot::default()
        };
        sink.record(&snapshot).unwrap();
        assert_eq!(sink.snapshots(), vec![snapshot]);
    }

    #[test]
    fn latency_snapshot_exposes_logarithmic_percentiles() {
        let diagnostics = LatencyDiagnostics::default();
        diagnostics.record(Duration::from_nanos(10));
        diagnostics.record(Duration::from_nanos(20));
        diagnostics.record(Duration::from_nanos(30));
        diagnostics.record(Duration::from_nanos(40));
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.samples, 4);
        assert_eq!(snapshot.total_nanos, 100);
        assert_eq!(snapshot.max_nanos, 40);
        assert_eq!(snapshot.p50_nanos, Some(32));
        assert_eq!(snapshot.p95_nanos, Some(64));
        assert_eq!(snapshot.p99_nanos, Some(64));
    }

    #[test]
    fn jsonl_sink_rotates_without_serializing_sensitive_fields() {
        let temporary = tempdir().unwrap();
        let path = temporary
            .path()
            .join("profile-a")
            .join("host-diagnostics.jsonl");
        let sink = RotatingJsonlDiagnosticsSink::new(path.clone(), 64);
        let snapshot = HostDiagnosticsSnapshot {
            schema_version: DIAGNOSTICS_SCHEMA_VERSION,
            captured_at_millis: 1,
            host_epoch: 7,
            ..HostDiagnosticsSnapshot::default()
        };
        sink.record(&snapshot).unwrap();
        sink.record(&snapshot).unwrap();
        assert!(path.exists());
        assert!(sink.rotated_path().exists());
        let current = fs::read_to_string(path).unwrap();
        assert!(current.contains("\"host_epoch\":7"));
        assert!(!current.contains("input"));
        assert!(!current.contains("environment"));
        assert!(!current.contains("clipboard"));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn process_sampler_reports_current_thread_count() {
        let mut sampler = ProcessDiagnosticsSampler::default();
        assert!(sampler.sample().thread_count.is_some_and(|count| count > 0));
    }
}
