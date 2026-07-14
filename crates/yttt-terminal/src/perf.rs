//! Low-overhead, opt-in terminal pipeline performance instrumentation.

#[cfg(feature = "perf-metrics")]
mod enabled {
    use bytes::Bytes;

    use hdrhistogram::Histogram;
    use parking_lot::{Mutex, RwLock};
    use serde::Serialize;
    use std::collections::VecDeque;
    use std::env;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex as StdMutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const MAX_RECORDED_NANOS: u64 = 120_000_000_000;
    const MAX_PENDING_INPUTS: usize = 4096;
    const MAX_PENDING_PARSER_EVENTS: usize = 16_384;
    const MAX_INPUT_CORRELATION_AGE: Duration = Duration::from_secs(30);

    const MAX_PENDING_IME_EVENTS: usize = 4096;
    const READ_QUEUE_CAPACITY: usize = 8;

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct InputPerformanceSample {
        sequence: u64,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct DurationDistribution {
        pub samples: u64,
        pub min_ms: Option<f64>,
        pub mean_ms: Option<f64>,
        pub p50_ms: Option<f64>,
        pub p90_ms: Option<f64>,
        pub p95_ms: Option<f64>,
        pub p99_ms: Option<f64>,
        pub max_ms: Option<f64>,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct SlowFrameCounts {
        pub over_8_33_ms: u64,
        pub over_16_67_ms: u64,
        pub over_33_3_ms: u64,
        pub over_50_ms: u64,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct TerminalPerformanceCounters {
        pub bytes_read: u64,
        pub parser_batches: u64,
        pub read_queue_current: usize,
        pub read_queue_high_water: usize,
        pub read_queue_capacity: usize,
        pub input_events: u64,
        pub painted_frames: u64,
        pub redraw_requests: u64,
        pub redraw_signals: u64,
        pub redraws_coalesced: u64,
        pub dropped_correlations: u64,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct TerminalLatencyMetrics {
        pub parser_batch_ms: DurationDistribution,
        pub parser_lock_wait_ms: DurationDistribution,
        pub parser_advance_ms: DurationDistribution,
        pub parser_to_prepaint_ms: DurationDistribution,
        pub prepaint_ms: DurationDistribution,
        pub paint_ms: DurationDistribution,
        pub paint_frame_interval_ms: DurationDistribution,
        pub input_to_pty_write_ms: DurationDistribution,
        pub input_to_echo_parse_ms: DurationDistribution,
        pub input_to_first_paint_after_echo_ms: DurationDistribution,

        pub ime_preedit_to_paint_ms: DurationDistribution,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct TerminalPerformanceSemantics {
        pub frame_interval: &'static str,
        pub input_to_parser: &'static str,
        pub input_to_paint: &'static str,
        pub presentation: &'static str,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct TerminalPerformanceSnapshot {
        pub schema_version: u32,
        pub elapsed_seconds: f64,
        pub paint_fps: f64,
        pub counters: TerminalPerformanceCounters,
        pub latencies: TerminalLatencyMetrics,
        pub slow_paint_frame_intervals: SlowFrameCounts,
        pub semantics: TerminalPerformanceSemantics,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct TerminalPerformanceDocument {
        pub label: String,
        pub scenario: String,
        pub phase: &'static str,
        pub generated_at_unix_ms: u64,
        pub metrics: TerminalPerformanceSnapshot,
    }

    struct DurationMetric {
        histogram: Mutex<Histogram<u64>>,
    }

    impl DurationMetric {
        fn new() -> Self {
            Self {
                histogram: Mutex::new(
                    Histogram::new_with_bounds(1, MAX_RECORDED_NANOS, 3)
                        .expect("terminal performance histogram bounds must be valid"),
                ),
            }
        }

        fn record(&self, duration: Duration) {
            let nanos = duration.as_nanos().clamp(1, MAX_RECORDED_NANOS as u128) as u64;
            self.histogram.lock().saturating_record(nanos);
        }

        fn snapshot(&self) -> DurationDistribution {
            distribution(&self.histogram.lock())
        }

        fn slow_frame_counts(&self) -> SlowFrameCounts {
            let histogram = self.histogram.lock();
            SlowFrameCounts {
                over_8_33_ms: count_over(&histogram, 8_330_000),
                over_16_67_ms: count_over(&histogram, 16_670_000),
                over_33_3_ms: count_over(&histogram, 33_300_000),
                over_50_ms: count_over(&histogram, 50_000_000),
            }
        }

        fn clear(&self) {
            self.histogram.lock().clear();
        }
    }

    fn count_over(histogram: &Histogram<u64>, threshold: u64) -> u64 {
        histogram.count_between(threshold.saturating_add(1), u64::MAX)
    }

    fn distribution(histogram: &Histogram<u64>) -> DurationDistribution {
        let samples = histogram.len();
        let value = |nanos: u64| Some(nanos as f64 / 1_000_000.0);
        if samples == 0 {
            return DurationDistribution {
                samples,
                min_ms: None,
                mean_ms: None,
                p50_ms: None,
                p90_ms: None,
                p95_ms: None,
                p99_ms: None,
                max_ms: None,
            };
        }

        DurationDistribution {
            samples,
            min_ms: value(histogram.min()),
            mean_ms: Some(histogram.mean() / 1_000_000.0),
            p50_ms: value(histogram.value_at_quantile(0.50)),
            p90_ms: value(histogram.value_at_quantile(0.90)),
            p95_ms: value(histogram.value_at_quantile(0.95)),
            p99_ms: value(histogram.value_at_quantile(0.99)),
            max_ms: value(histogram.max()),
        }
    }

    struct ParserCompletion {
        generation: u64,
        completed_at: Instant,
    }

    struct PendingInput {
        sequence: u64,
        started_at: Instant,
        written_at: Option<Instant>,
        expected_echo: Bytes,
        match_prefix: Vec<usize>,
        match_offset: usize,
        parsed_generation: Option<u64>,
    }
    fn match_prefix(bytes: &[u8]) -> Vec<usize> {
        let mut prefix = vec![0; bytes.len()];
        let mut matched = 0;
        for index in 1..bytes.len() {
            while matched > 0 && bytes[index] != bytes[matched] {
                matched = prefix[matched - 1];
            }
            if bytes[index] == bytes[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        prefix
    }

    fn advance_echo_match(input: &mut PendingInput, bytes: &[u8]) -> bool {
        if input.expected_echo.is_empty() {
            return true;
        }
        for &byte in bytes {
            while input.match_offset > 0 && byte != input.expected_echo[input.match_offset] {
                input.match_offset = input.match_prefix[input.match_offset - 1];
            }
            if byte == input.expected_echo[input.match_offset] {
                input.match_offset += 1;
                if input.match_offset == input.expected_echo.len() {
                    return true;
                }
            }
        }
        false
    }

    struct PerformanceState {
        started_at: RwLock<Instant>,
        finished_at: RwLock<Option<Instant>>,

        bytes_read: AtomicU64,
        parser_batches: AtomicU64,
        read_queue_current: AtomicUsize,
        read_queue_high_water: AtomicUsize,
        input_events: AtomicU64,
        painted_frames: AtomicU64,
        redraw_requests: AtomicU64,
        redraw_signals: AtomicU64,
        dropped_correlations: AtomicU64,
        parser_generation: AtomicU64,
        input_sequence: AtomicU64,
        parser_completions: Mutex<VecDeque<ParserCompletion>>,
        pending_inputs: Mutex<VecDeque<PendingInput>>,
        pending_ime_preedits: Mutex<VecDeque<Instant>>,
        last_paint_at: Mutex<Option<Instant>>,
        parser_batch: DurationMetric,
        parser_lock_wait: DurationMetric,
        parser_advance: DurationMetric,
        parser_to_prepaint: DurationMetric,
        prepaint: DurationMetric,
        paint: DurationMetric,
        paint_frame_interval: DurationMetric,
        input_to_pty_write: DurationMetric,
        input_to_echo_parse: DurationMetric,
        input_to_first_paint_after_echo: DurationMetric,

        ime_preedit_to_paint: DurationMetric,
    }

    impl PerformanceState {
        fn new() -> Self {
            Self {
                started_at: RwLock::new(Instant::now()),
                finished_at: RwLock::new(None),

                bytes_read: AtomicU64::new(0),
                parser_batches: AtomicU64::new(0),
                read_queue_current: AtomicUsize::new(0),
                read_queue_high_water: AtomicUsize::new(0),
                input_events: AtomicU64::new(0),
                painted_frames: AtomicU64::new(0),
                redraw_requests: AtomicU64::new(0),
                redraw_signals: AtomicU64::new(0),
                dropped_correlations: AtomicU64::new(0),
                parser_generation: AtomicU64::new(0),
                input_sequence: AtomicU64::new(0),
                parser_completions: Mutex::new(VecDeque::new()),
                pending_inputs: Mutex::new(VecDeque::new()),
                pending_ime_preedits: Mutex::new(VecDeque::new()),
                last_paint_at: Mutex::new(None),
                parser_batch: DurationMetric::new(),
                parser_lock_wait: DurationMetric::new(),
                parser_advance: DurationMetric::new(),
                parser_to_prepaint: DurationMetric::new(),
                prepaint: DurationMetric::new(),
                paint: DurationMetric::new(),
                paint_frame_interval: DurationMetric::new(),
                input_to_pty_write: DurationMetric::new(),
                input_to_echo_parse: DurationMetric::new(),
                input_to_first_paint_after_echo: DurationMetric::new(),

                ime_preedit_to_paint: DurationMetric::new(),
            }
        }
    }

    #[derive(Clone)]
    pub struct TerminalPerformanceHandle {
        state: Arc<PerformanceState>,
    }

    impl Default for TerminalPerformanceHandle {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TerminalPerformanceHandle {
        pub(crate) fn new() -> Self {
            Self {
                state: Arc::new(PerformanceState::new()),
            }
        }

        pub(crate) fn record_read(&self, bytes: usize) {
            self.state
                .bytes_read
                .fetch_add(bytes as u64, Ordering::Relaxed);
        }

        pub(crate) fn set_read_queue_depth(&self, depth: usize) {
            self.state
                .read_queue_current
                .store(depth, Ordering::Relaxed);
            self.state
                .read_queue_high_water
                .fetch_max(depth, Ordering::Relaxed);
        }

        pub(crate) fn begin_input(&self, bytes: &Bytes) -> InputPerformanceSample {
            let sequence = self.state.input_sequence.fetch_add(1, Ordering::Relaxed) + 1;
            self.state.input_events.fetch_add(1, Ordering::Relaxed);
            let mut pending = self.state.pending_inputs.lock();
            if pending.len() >= MAX_PENDING_INPUTS {
                pending.pop_front();
                self.state
                    .dropped_correlations
                    .fetch_add(1, Ordering::Relaxed);
            }
            pending.push_back(PendingInput {
                sequence,
                started_at: Instant::now(),
                written_at: None,
                expected_echo: bytes.clone(),
                match_prefix: match_prefix(bytes),
                match_offset: 0,
                parsed_generation: None,
            });
            InputPerformanceSample { sequence }
        }

        pub(crate) fn cancel_input(&self, sample: InputPerformanceSample) {
            let mut pending = self.state.pending_inputs.lock();
            if let Some(index) = pending
                .iter()
                .position(|input| input.sequence == sample.sequence)
            {
                pending.remove(index);
            }
        }

        pub(crate) fn record_input_written(
            &self,
            sample: InputPerformanceSample,
            completed_at: Instant,
        ) {
            let started_at = {
                let mut pending = self.state.pending_inputs.lock();
                pending
                    .iter_mut()
                    .find(|input| input.sequence == sample.sequence)
                    .map(|input| {
                        input.written_at = Some(completed_at);
                        input.started_at
                    })
            };
            if let Some(started_at) = started_at {
                self.state
                    .input_to_pty_write
                    .record(completed_at.saturating_duration_since(started_at));
            }
        }

        pub(crate) fn record_parser_batch(
            &self,
            total: Duration,
            lock_wait: Duration,
            advance: Duration,
            completed_at: Instant,
            bytes: &[u8],
        ) -> u64 {
            self.state.parser_batches.fetch_add(1, Ordering::Relaxed);
            self.state.parser_batch.record(total);
            self.state.parser_lock_wait.record(lock_wait);
            self.state.parser_advance.record(advance);

            let generation = self.state.parser_generation.fetch_add(1, Ordering::AcqRel) + 1;
            {
                let mut completions = self.state.parser_completions.lock();
                if completions.len() >= MAX_PENDING_PARSER_EVENTS {
                    completions.pop_front();
                    self.state
                        .dropped_correlations
                        .fetch_add(1, Ordering::Relaxed);
                }
                completions.push_back(ParserCompletion {
                    generation,
                    completed_at,
                });
            }

            let mut parser_latencies = Vec::new();
            {
                let mut pending = self.state.pending_inputs.lock();
                for input in pending.iter_mut() {
                    if input.parsed_generation.is_none() && advance_echo_match(input, bytes) {
                        input.parsed_generation = Some(generation);
                        parser_latencies
                            .push(completed_at.saturating_duration_since(input.started_at));
                    }
                }
            }
            for latency in parser_latencies {
                self.state.input_to_echo_parse.record(latency);
            }
            generation
        }

        pub(crate) fn parser_generation(&self) -> u64 {
            self.state.parser_generation.load(Ordering::Acquire)
        }

        pub(crate) fn record_prepaint(
            &self,
            parser_generation: u64,
            started_at: Instant,
            duration: Duration,
        ) {
            self.state.prepaint.record(duration);
            let mut latencies = Vec::new();
            {
                let mut completions = self.state.parser_completions.lock();
                while completions
                    .front()
                    .is_some_and(|event| event.generation <= parser_generation)
                {
                    if let Some(event) = completions.pop_front() {
                        latencies.push(started_at.saturating_duration_since(event.completed_at));
                    }
                }
            }
            for latency in latencies {
                self.state.parser_to_prepaint.record(latency);
            }
        }

        pub(crate) fn record_paint(
            &self,
            parser_generation: u64,
            completed_at: Instant,
            duration: Duration,
        ) {
            self.state.paint.record(duration);
            self.state.painted_frames.fetch_add(1, Ordering::Relaxed);
            if let Some(previous) = self.state.last_paint_at.lock().replace(completed_at) {
                self.state
                    .paint_frame_interval
                    .record(completed_at.saturating_duration_since(previous));
            }

            let mut input_latencies = Vec::new();
            let mut dropped = 0_u64;
            {
                let mut pending = self.state.pending_inputs.lock();
                let mut index = 0;
                while index < pending.len() {
                    let input = &pending[index];
                    let parsed = input
                        .parsed_generation
                        .is_some_and(|generation| generation <= parser_generation);
                    let expired = completed_at.saturating_duration_since(input.started_at)
                        >= MAX_INPUT_CORRELATION_AGE;
                    if parsed {
                        let input = pending
                            .remove(index)
                            .expect("pending input index must remain valid");
                        input_latencies
                            .push(completed_at.saturating_duration_since(input.started_at));
                    } else if expired {
                        pending.remove(index);
                        dropped += 1;
                    } else {
                        index += 1;
                    }
                }
            }
            if dropped > 0 {
                self.state
                    .dropped_correlations
                    .fetch_add(dropped, Ordering::Relaxed);
            }
            for latency in input_latencies {
                self.state.input_to_first_paint_after_echo.record(latency);
            }

            let ime_preedits = {
                let mut pending = self.state.pending_ime_preedits.lock();
                pending.drain(..).collect::<Vec<_>>()
            };
            for started_at in ime_preedits {
                self.state
                    .ime_preedit_to_paint
                    .record(completed_at.saturating_duration_since(started_at));
            }
        }

        pub(crate) fn record_ime_preedit(&self) {
            let mut pending = self.state.pending_ime_preedits.lock();
            if pending.len() >= MAX_PENDING_IME_EVENTS {
                pending.pop_front();
                self.state
                    .dropped_correlations
                    .fetch_add(1, Ordering::Relaxed);
            }
            pending.push_back(Instant::now());
        }

        pub(crate) fn record_redraw_request(&self, signaled: bool) {
            self.state.redraw_requests.fetch_add(1, Ordering::Relaxed);
            if signaled {
                self.state.redraw_signals.fetch_add(1, Ordering::Relaxed);
            }
        }
        pub(crate) fn record_redraw_signal(&self) {
            self.state.redraw_signals.fetch_add(1, Ordering::Relaxed);
        }
        pub(crate) fn finish_measurement(&self) {
            let mut finished_at = self.state.finished_at.write();
            if finished_at.is_none() {
                *finished_at = Some(Instant::now());
            }
        }

        pub fn snapshot(&self) -> TerminalPerformanceSnapshot {
            let started_at = *self.state.started_at.read();
            let elapsed = self
                .state
                .finished_at
                .read()
                .map(|finished_at| finished_at.saturating_duration_since(started_at))
                .unwrap_or_else(|| started_at.elapsed());
            let elapsed_seconds = elapsed.as_secs_f64();

            let painted_frames = self.state.painted_frames.load(Ordering::Relaxed);
            let redraw_requests = self.state.redraw_requests.load(Ordering::Relaxed);
            let redraw_signals = self.state.redraw_signals.load(Ordering::Relaxed);
            TerminalPerformanceSnapshot {
                schema_version: 1,
                elapsed_seconds,
                paint_fps: if elapsed_seconds > 0.0 {
                    painted_frames as f64 / elapsed_seconds
                } else {
                    0.0
                },
                counters: TerminalPerformanceCounters {
                    bytes_read: self.state.bytes_read.load(Ordering::Relaxed),
                    parser_batches: self.state.parser_batches.load(Ordering::Relaxed),
                    read_queue_current: self.state.read_queue_current.load(Ordering::Relaxed),
                    read_queue_high_water: self.state.read_queue_high_water.load(Ordering::Relaxed),
                    read_queue_capacity: READ_QUEUE_CAPACITY,
                    input_events: self.state.input_events.load(Ordering::Relaxed),
                    painted_frames,
                    redraw_requests,
                    redraw_signals,
                    redraws_coalesced: redraw_requests.saturating_sub(redraw_signals),
                    dropped_correlations: self.state.dropped_correlations.load(Ordering::Relaxed),
                },
                latencies: TerminalLatencyMetrics {
                    parser_batch_ms: self.state.parser_batch.snapshot(),
                    parser_lock_wait_ms: self.state.parser_lock_wait.snapshot(),
                    parser_advance_ms: self.state.parser_advance.snapshot(),
                    parser_to_prepaint_ms: self.state.parser_to_prepaint.snapshot(),
                    prepaint_ms: self.state.prepaint.snapshot(),
                    paint_ms: self.state.paint.snapshot(),
                    paint_frame_interval_ms: self.state.paint_frame_interval.snapshot(),
                    input_to_pty_write_ms: self.state.input_to_pty_write.snapshot(),
                    input_to_echo_parse_ms: self.state.input_to_echo_parse.snapshot(),
                    input_to_first_paint_after_echo_ms: self
                        .state
                        .input_to_first_paint_after_echo
                        .snapshot(),

                    ime_preedit_to_paint_ms: self.state.ime_preedit_to_paint.snapshot(),
                },
                slow_paint_frame_intervals: self.state.paint_frame_interval.slow_frame_counts(),
                semantics: TerminalPerformanceSemantics {
                    frame_interval: "Time between completed terminal paint callbacks; this is not the Metal drawable presentation interval.",
                    input_to_parser: "Time from a GPUI input event until the parser observes the first subsequent PTY output occurrence of the exact submitted byte sequence; unmatched control input is omitted.",
                    input_to_paint: "Time to the first terminal paint whose snapshot includes the matched echo parser generation.",

                    presentation: "Actual GPU/display presentation is intentionally measured by the accompanying Metal System Trace capture.",
                },
            }
        }

        pub fn reset(&self) {
            *self.state.started_at.write() = Instant::now();
            *self.state.finished_at.write() = None;

            self.state.bytes_read.store(0, Ordering::Relaxed);
            self.state.parser_batches.store(0, Ordering::Relaxed);
            let current_depth = self.state.read_queue_current.load(Ordering::Relaxed);
            self.state
                .read_queue_high_water
                .store(current_depth, Ordering::Relaxed);
            self.state.input_events.store(0, Ordering::Relaxed);
            self.state.painted_frames.store(0, Ordering::Relaxed);
            self.state.redraw_requests.store(0, Ordering::Relaxed);
            self.state.redraw_signals.store(0, Ordering::Relaxed);
            self.state.dropped_correlations.store(0, Ordering::Relaxed);
            self.state.parser_completions.lock().clear();
            self.state.pending_inputs.lock().clear();
            self.state.pending_ime_preedits.lock().clear();
            *self.state.last_paint_at.lock() = None;
            self.state.parser_batch.clear();
            self.state.parser_lock_wait.clear();
            self.state.parser_advance.clear();
            self.state.parser_to_prepaint.clear();
            self.state.prepaint.clear();
            self.state.paint.clear();
            self.state.paint_frame_interval.clear();
            self.state.input_to_pty_write.clear();
            self.state.input_to_echo_parse.clear();
            self.state.input_to_first_paint_after_echo.clear();

            self.state.ime_preedit_to_paint.clear();
        }

        pub fn spawn_reporter_from_env(&self) -> io::Result<Option<TerminalPerformanceReporter>> {
            let Some(path) = env::var_os("YTTT_TERMINAL_PERF_OUTPUT").map(PathBuf::from) else {
                return Ok(None);
            };
            let label = env::var("YTTT_TERMINAL_PERF_LABEL")
                .unwrap_or_else(|_| "yttt-terminal".to_string());
            let scenario =
                env::var("YTTT_TERMINAL_PERF_SCENARIO").unwrap_or_else(|_| "manual".to_string());
            let interval = env_duration_ms("YTTT_TERMINAL_PERF_REPORT_INTERVAL_MS", 250);
            let warmup = env_duration_seconds("YTTT_TERMINAL_PERF_WARMUP_SECONDS", 0.0);
            let measurement_duration =
                env_optional_duration_seconds("YTTT_TERMINAL_PERF_DURATION_SECONDS");

            let start_file = env::var_os("YTTT_TERMINAL_PERF_START_FILE").map(PathBuf::from);
            let ready_file = env::var_os("YTTT_TERMINAL_PERF_READY_FILE").map(PathBuf::from);
            let reporter = TerminalPerformanceReporter::spawn(
                self.clone(),
                path,
                label,
                scenario,
                interval,
                warmup,
                measurement_duration,
                start_file,
            )?;
            if let Some(ready_file) = ready_file {
                if let Some(parent) = ready_file.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(ready_file, [])?;
            }
            Ok(Some(reporter))
        }
    }

    fn env_duration_ms(name: &str, default: u64) -> Duration {
        env::var(name)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .filter(|duration| !duration.is_zero())
            .unwrap_or_else(|| Duration::from_millis(default))
    }

    fn env_duration_seconds(name: &str, default: f64) -> Duration {
        env::var(name)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(Duration::from_secs_f64)
            .unwrap_or_else(|| Duration::from_secs_f64(default))
    }

    fn env_optional_duration_seconds(name: &str) -> Option<Duration> {
        env::var(name)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(Duration::from_secs_f64)
    }

    pub struct TerminalPerformanceReporter {
        stop: Arc<(StdMutex<bool>, Condvar)>,
        thread: Option<JoinHandle<()>>,
    }

    impl TerminalPerformanceReporter {
        fn spawn(
            performance: TerminalPerformanceHandle,
            path: PathBuf,
            label: String,
            scenario: String,
            interval: Duration,
            warmup: Duration,
            measurement_duration: Option<Duration>,

            start_file: Option<PathBuf>,
        ) -> io::Result<Self> {
            let waiting_for_start = start_file.as_ref().is_some_and(|path| !path.is_file());

            write_document(
                &path,
                &TerminalPerformanceDocument {
                    label: label.clone(),
                    scenario: scenario.clone(),
                    phase: if waiting_for_start {
                        "waiting_for_workload"
                    } else if warmup.is_zero() {
                        "measuring"
                    } else {
                        "warming_up"
                    },

                    generated_at_unix_ms: unix_time_ms(),
                    metrics: performance.snapshot(),
                },
            )?;

            let stop = Arc::new((StdMutex::new(false), Condvar::new()));
            let thread_stop = stop.clone();
            let thread = thread::Builder::new()
                .name("yttt-terminal-perf-reporter".to_string())
                .spawn(move || {
                    let mut measurement_started_at = (!waiting_for_start).then(Instant::now);
                    let mut measuring = measurement_started_at.is_some() && warmup.is_zero();
                    let mut finished = false;
                    let deadline_from_now = || {
                        measurement_duration
                            .and_then(|duration| Instant::now().checked_add(duration))
                    };
                    let mut measurement_deadline = None;
                    let mut finished_snapshot = None;

                    if measuring {
                        performance.reset();
                        measurement_deadline = deadline_from_now();
                    }
                    loop {
                        let (lock, ready) = &*thread_stop;
                        let stopped = lock.lock().unwrap_or_else(|error| error.into_inner());
                        let mut wait_duration = if measuring || finished {
                            interval
                        } else {
                            interval.min(Duration::from_millis(10))
                        };
                        if let Some(deadline) = measurement_deadline {
                            wait_duration = wait_duration
                                .min(deadline.saturating_duration_since(Instant::now()));
                        }
                        let (stopped, _) = ready
                            .wait_timeout(stopped, wait_duration)
                            .unwrap_or_else(|error| error.into_inner());
                        let should_stop = *stopped;
                        drop(stopped);

                        if measurement_started_at.is_none()
                            && start_file.as_ref().is_none_or(|path| path.is_file())
                        {
                            measurement_started_at = Some(Instant::now());
                            if warmup.is_zero() {
                                performance.reset();
                                measuring = true;
                                measurement_deadline = deadline_from_now();
                            }
                        }
                        if !measuring
                            && !finished
                            && measurement_started_at
                                .is_some_and(|started_at| started_at.elapsed() >= warmup)
                        {
                            performance.reset();
                            measuring = true;
                            measurement_deadline = deadline_from_now();
                        }
                        if measuring
                            && measurement_deadline
                                .is_some_and(|deadline| Instant::now() >= deadline)
                        {
                            performance.finish_measurement();
                            finished_snapshot = Some(performance.snapshot());

                            measuring = false;
                            finished = true;
                        }
                        if should_stop && !finished {
                            performance.finish_measurement();
                            finished_snapshot = Some(performance.snapshot());
                            measuring = false;
                            finished = true;
                        }

                        if !measuring && !finished && !should_stop {
                            continue;
                        }

                        let phase = if measurement_started_at.is_none() {
                            "waiting_for_workload"
                        } else if finished {
                            "finished"
                        } else if measuring {
                            "measuring"
                        } else {
                            "warming_up"
                        };
                        let document = TerminalPerformanceDocument {
                            label: label.clone(),
                            scenario: scenario.clone(),
                            phase,
                            generated_at_unix_ms: unix_time_ms(),
                            metrics: finished_snapshot
                                .clone()
                                .unwrap_or_else(|| performance.snapshot()),
                        };
                        if let Err(error) = write_document(&path, &document) {
                            eprintln!(
                                "failed to write terminal performance report {}: {error}",
                                path.display()
                            );
                        }
                        if should_stop {
                            break;
                        }
                    }
                })?;
            Ok(Self {
                stop,
                thread: Some(thread),
            })
        }
    }

    impl Drop for TerminalPerformanceReporter {
        fn drop(&mut self) {
            let (lock, ready) = &*self.stop;
            *lock.lock().unwrap_or_else(|error| error.into_inner()) = true;
            ready.notify_all();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn unix_time_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    fn write_document(path: &Path, document: &TerminalPerformanceDocument) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("terminal-performance.json");
        let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
        let mut bytes = serde_json::to_vec_pretty(document).map_err(io::Error::other)?;
        bytes.push(b'\n');
        fs::write(&temporary, bytes)?;
        fs::rename(temporary, path)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn snapshot_reports_correlated_pipeline_latencies() {
            let performance = TerminalPerformanceHandle::new();
            let echoed = Bytes::from_static("中".as_bytes());
            let input = performance.begin_input(&echoed);
            let written_at = Instant::now();
            performance.record_input_written(input, written_at);
            let generation = performance.record_parser_batch(
                Duration::from_micros(30),
                Duration::from_micros(5),
                Duration::from_micros(20),
                Instant::now(),
                "prefix-中-suffix".as_bytes(),
            );

            let prepaint_started = Instant::now();
            performance.record_prepaint(generation, prepaint_started, Duration::from_micros(40));
            performance.record_paint(generation, Instant::now(), Duration::from_micros(50));

            let snapshot = performance.snapshot();
            assert_eq!(snapshot.counters.input_events, 1);
            assert_eq!(snapshot.counters.parser_batches, 1);
            assert_eq!(snapshot.counters.painted_frames, 1);
            assert_eq!(snapshot.latencies.input_to_pty_write_ms.samples, 1);
            assert_eq!(snapshot.latencies.input_to_echo_parse_ms.samples, 1);
            assert_eq!(
                snapshot
                    .latencies
                    .input_to_first_paint_after_echo_ms
                    .samples,
                1
            );

            assert_eq!(snapshot.latencies.parser_to_prepaint_ms.samples, 1);
        }

        #[test]
        fn reset_starts_a_fresh_measurement_window() {
            let performance = TerminalPerformanceHandle::new();
            performance.record_read(128);
            performance.record_redraw_request(true);
            performance.record_redraw_request(false);
            performance.record_ime_preedit();
            performance.record_paint(0, Instant::now(), Duration::from_millis(1));
            performance.reset();

            let snapshot = performance.snapshot();
            assert_eq!(snapshot.counters.bytes_read, 0);
            assert_eq!(snapshot.counters.redraw_requests, 0);
            assert_eq!(snapshot.counters.painted_frames, 0);
            assert_eq!(snapshot.latencies.paint_ms.samples, 0);
            assert_eq!(snapshot.latencies.ime_preedit_to_paint_ms.samples, 0);
        }
        #[test]
        fn reporter_waits_for_workload_gate_before_resetting_metrics() {
            let unique = format!(
                "yttt-terminal-perf-{}-{}",
                std::process::id(),
                unix_time_ms()
            );
            let directory = std::env::temp_dir().join(unique);
            let report_path = directory.join("metrics.json");
            let start_file = directory.join("workload.start");
            let performance = TerminalPerformanceHandle::new();
            performance.record_read(42);
            let reporter = TerminalPerformanceReporter::spawn(
                performance.clone(),
                report_path.clone(),
                "test".to_string(),
                "gate".to_string(),
                Duration::from_millis(10),
                Duration::ZERO,
                None,
                Some(start_file.clone()),
            )
            .unwrap();

            thread::sleep(Duration::from_millis(75));
            assert_eq!(performance.snapshot().counters.bytes_read, 42);
            let initial: serde_json::Value =
                serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
            assert_eq!(initial["phase"], "waiting_for_workload");

            fs::write(&start_file, []).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while performance.snapshot().counters.bytes_read != 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(performance.snapshot().counters.bytes_read, 0);

            drop(reporter);
            fs::remove_dir_all(directory).unwrap();
        }
        #[test]
        fn reporter_freezes_metrics_at_configured_duration() {
            let unique = format!(
                "yttt-terminal-perf-duration-{}-{}",
                std::process::id(),
                unix_time_ms()
            );
            let directory = std::env::temp_dir().join(unique);
            let report_path = directory.join("metrics.json");
            let performance = TerminalPerformanceHandle::new();
            let reporter = TerminalPerformanceReporter::spawn(
                performance.clone(),
                report_path.clone(),
                "test".to_string(),
                "duration".to_string(),
                Duration::from_millis(200),
                Duration::ZERO,
                Some(Duration::from_millis(40)),
                None,
            )
            .unwrap();

            let deadline = Instant::now() + Duration::from_secs(2);
            let elapsed = loop {
                let document: serde_json::Value =
                    serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
                if document["phase"] == "finished" {
                    break document["metrics"]["elapsed_seconds"].as_f64().unwrap();
                }
                assert!(Instant::now() < deadline, "reporter did not finish");
                thread::sleep(Duration::from_millis(5));
            };
            assert!((0.03..=0.08).contains(&elapsed), "{elapsed}");
            thread::sleep(Duration::from_millis(40));
            assert_eq!(performance.snapshot().elapsed_seconds, elapsed);

            drop(reporter);
            fs::remove_dir_all(directory).unwrap();
        }
    }
}

#[cfg(feature = "perf-metrics")]
pub(crate) use enabled::InputPerformanceSample;
#[cfg(feature = "perf-metrics")]
pub use enabled::{
    DurationDistribution, SlowFrameCounts, TerminalLatencyMetrics, TerminalPerformanceCounters,
    TerminalPerformanceDocument, TerminalPerformanceHandle, TerminalPerformanceReporter,
    TerminalPerformanceSemantics, TerminalPerformanceSnapshot,
};

#[cfg(not(feature = "perf-metrics"))]
mod disabled {
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct InputPerformanceSample;

    #[derive(Clone, Default)]
    pub(crate) struct TerminalPerformanceHandle;

    impl TerminalPerformanceHandle {
        pub(crate) fn new() -> Self {
            Self
        }

        pub(crate) fn record_read(&self, _bytes: usize) {}

        pub(crate) fn set_read_queue_depth(&self, _depth: usize) {}

        pub(crate) fn begin_input(&self, _bytes: &bytes::Bytes) -> InputPerformanceSample {
            InputPerformanceSample
        }

        pub(crate) fn cancel_input(&self, _sample: InputPerformanceSample) {}

        pub(crate) fn record_input_written(
            &self,
            _sample: InputPerformanceSample,
            _completed_at: Instant,
        ) {
        }

        pub(crate) fn record_parser_batch(
            &self,
            _total: Duration,
            _lock_wait: Duration,
            _advance: Duration,
            _completed_at: Instant,
            _bytes: &[u8],
        ) -> u64 {
            0
        }

        pub(crate) fn parser_generation(&self) -> u64 {
            0
        }

        pub(crate) fn record_prepaint(
            &self,
            _parser_generation: u64,
            _started_at: Instant,
            _duration: Duration,
        ) {
        }

        pub(crate) fn record_paint(
            &self,
            _parser_generation: u64,
            _completed_at: Instant,
            _duration: Duration,
        ) {
        }

        pub(crate) fn record_ime_preedit(&self) {}
        pub(crate) fn finish_measurement(&self) {}

        pub(crate) fn record_redraw_request(&self, _signaled: bool) {}

        pub(crate) fn record_redraw_signal(&self) {}
    }
}

#[cfg(not(feature = "perf-metrics"))]
pub(crate) use disabled::{InputPerformanceSample, TerminalPerformanceHandle};
