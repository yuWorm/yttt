//! The log setup the Host process and the desktop share: one `tracing`
//! subscriber whose detail follows `[diagnostics] verbose_logs` in
//! `settings.toml` and can change while the process runs.
//!
//! Normal detail records what explains a failure afterwards: connections and
//! why they ended, slow or failed requests, recoveries. Verbose detail adds
//! every request with its timing, heartbeats that come back slowly, Agent
//! snapshot pages and the transports' own debug events.

use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

use parking_lot::Mutex;
use tracing_subscriber::{
    EnvFilter, Registry, fmt, layer::SubscriberExt as _, reload, util::SubscriberInitExt as _,
};

/// Filter directives that replace the setting, such as
/// `yttt_host=trace,iroh=debug`.
pub const LOG_FILTER_ENV: &str = "YTTT_LOG";

/// The crates whose detail the setting raises; everything else stays at
/// warnings, or at info while verbose.
const OUR_CRATES: &[&str] = &[
    "yttt",
    "yttt_host",
    "yttt_client_core",
    "yttt_transport",
    "yttt_transport_iroh",
    "yttt_transport_local",
    "yttt_transport_tls",
    "yttt_ssh",
];

static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
/// The detail in effect: 0 until installed, then 1 normal or 2 verbose.
static DETAIL: AtomicU8 = AtomicU8::new(0);

/// The directives for normal or verbose detail.
pub fn filter_directives(verbose: bool) -> String {
    let (others, ours) = if verbose {
        ("info", "debug")
    } else {
        ("warn", "info")
    };
    std::iter::once(others.to_string())
        .chain(OUR_CRATES.iter().map(|name| format!("{name}={ours}")))
        .collect::<Vec<_>>()
        .join(",")
}

fn filter(verbose: bool) -> EnvFilter {
    std::env::var(LOG_FILTER_ENV)
        .ok()
        .and_then(|directives| EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| EnvFilter::new(filter_directives(verbose)))
}

/// Installs this process's subscriber, writing lines to `writer`, each
/// marked with `process` and the process id, since processes share a file.
/// Only the first call in a process installs one.
pub fn install<W>(writer: W, process: &'static str, verbose: bool)
where
    W: for<'writer> fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    let (filter, handle) = reload::Layer::new(filter(verbose));
    let lines = fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_timer(LocalTime {
            process,
            pid: std::process::id(),
        });
    if tracing_subscriber::registry()
        .with(filter)
        .with(lines)
        .try_init()
        .is_ok()
    {
        let _ = FILTER.set(handle);
        DETAIL.store(detail(verbose), Ordering::Relaxed);
    }
}

/// Switches between normal and verbose detail; `YTTT_LOG` still wins.
/// Cheap when the detail is unchanged, so callers may repeat it freely.
pub fn set_verbose(verbose: bool) {
    let Some(handle) = FILTER.get() else {
        return;
    };
    if DETAIL.load(Ordering::Relaxed) == detail(verbose)
        || DETAIL.swap(detail(verbose), Ordering::Relaxed) == detail(verbose)
    {
        return;
    }
    let _ = handle.reload(filter(verbose));
    tracing::info!(verbose, "log detail changed");
}

fn detail(verbose: bool) -> u8 {
    if verbose { 2 } else { 1 }
}

/// `[diagnostics] verbose_logs` in `<config_root>/settings.toml`; off when
/// the file or the key is missing or unreadable.
pub fn verbose_setting(config_root: &Path) -> bool {
    std::fs::read_to_string(config_root.join("settings.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| table.get("diagnostics")?.get("verbose_logs")?.as_bool())
        .unwrap_or(false)
}

/// Timestamps in local time, as the logs always had, followed by the
/// process that wrote the line, such as `remote-window#8328`.
struct LocalTime {
    process: &'static str,
    pid: u32,
}

impl fmt::time::FormatTime for LocalTime {
    fn format_time(&self, writer: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(
            writer,
            "{} {}#{}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z"),
            self.process,
            self.pid
        )
    }
}

/// A log file that moves to `<name>.1`, replacing an older one, once it
/// outgrows `max_bytes`. Each line is appended whole, so processes sharing
/// the file do not interleave within a line.
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    lock: Mutex<()>,
}

impl RotatingFile {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes,
            lock: Mutex::new(()),
        }
    }

    pub fn append(&self, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write as _;
        let _held = self.lock.lock();
        if std::fs::metadata(&self.path).is_ok_and(|metadata| metadata.len() > self.max_bytes) {
            let mut rotated = self.path.clone().into_os_string();
            rotated.push(".1");
            std::fs::rename(&self.path, rotated)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(bytes)
    }
}

impl<'writer> fmt::MakeWriter<'writer> for RotatingFile {
    type Writer = RotatingLine<'writer>;

    fn make_writer(&'writer self) -> Self::Writer {
        RotatingLine {
            file: self,
            line: Vec::new(),
        }
    }
}

/// One event's text, appended when the subscriber is done writing it.
pub struct RotatingLine<'file> {
    file: &'file RotatingFile,
    line: Vec<u8>,
}

impl io::Write for RotatingLine<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RotatingLine<'_> {
    fn drop(&mut self) {
        if !self.line.is_empty() {
            let _ = self.file.append(&self.line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbose_detail_raises_our_crates_and_keeps_others_quieter() {
        let normal = filter_directives(false);
        assert!(normal.starts_with("warn,"));
        assert!(normal.contains("yttt_host=info"));
        let verbose = filter_directives(true);
        assert!(verbose.starts_with("info,"));
        assert!(verbose.contains("yttt_client_core=debug"));
        for directives in [normal, verbose] {
            EnvFilter::try_new(directives).unwrap();
        }
    }

    #[test]
    fn the_setting_is_read_from_settings_toml() {
        let config = tempfile::tempdir().unwrap();
        assert!(!verbose_setting(config.path()));
        std::fs::write(
            config.path().join("settings.toml"),
            "[diagnostics]\nverbose_logs = true\n",
        )
        .unwrap();
        assert!(verbose_setting(config.path()));
        std::fs::write(config.path().join("settings.toml"), "not toml [").unwrap();
        assert!(!verbose_setting(config.path()));
    }

    /// The only test in this binary that installs the process subscriber.
    #[test]
    fn the_switch_raises_the_detail_of_the_installed_subscriber() {
        if std::env::var_os(LOG_FILTER_ENV).is_some() {
            return;
        }
        let logs = tempfile::tempdir().unwrap();
        let path = logs.path().join("host.log");
        install(RotatingFile::new(path.clone(), 1 << 20), "test", false);
        tracing::info!("normal line");
        tracing::debug!("verbose line one");
        set_verbose(true);
        tracing::debug!("verbose line two");
        set_verbose(false);
        tracing::debug!("verbose line three");
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(
            log.contains(&format!(
                " test#{}  INFO yttt_host::logging::tests: normal line",
                std::process::id()
            )),
            "{log}"
        );
        assert!(!log.contains("verbose line one"), "{log}");
        assert!(log.contains("verbose line two"), "{log}");
        assert!(!log.contains("verbose line three"), "{log}");
        assert_eq!(log.matches("log detail changed").count(), 2, "{log}");
    }

    #[test]
    fn a_log_file_rotates_once_it_outgrows_its_bound() {
        use std::io::Write as _;
        let logs = tempfile::tempdir().unwrap();
        let path = logs.path().join("client.log");
        std::fs::write(&path, vec![b'x'; 65]).unwrap();
        let file = RotatingFile::new(path.clone(), 64);
        {
            let mut line = fmt::MakeWriter::make_writer(&file);
            line.write_all(b"next\n").unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next\n");
        assert_eq!(
            std::fs::read(logs.path().join("client.log.1"))
                .unwrap()
                .len(),
            65
        );
    }
}
