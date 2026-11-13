//! The phone's connection log: what the core says about connecting,
//! reconnecting, paths and relay tickets, kept in a small rotating file in
//! the app's own directory, so a slow or failed connect can be read
//! afterwards. `YTTT_LOG`, as `RUST_LOG`, also sends lines to stderr for a
//! debug run.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing_subscriber::{
    EnvFilter, Layer as _, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

const LOG_FILE: &str = "connection.log";
/// Past this size the log moves to `connection.log.1`, replacing an older one.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// What the file keeps: the app's own lines, and iroh's warnings.
const FILE_FILTER: &str = "warn,yttt_mobile_core=info,yttt_client_core=info,yttt_transport_iroh=info,yttt_control_client=info";
/// How much of the log `read` returns, from its end.
const READ_BYTES: usize = 256 * 1024;

/// A file that starts over when it grows past [`MAX_LOG_BYTES`].
pub struct RotatingFile {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl RotatingFile {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(LOG_FILE),
            file: Mutex::new(None),
        }
    }

    fn write_line(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if file.is_none() {
            *file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        let open = file.as_mut().expect("opened above");
        if open.metadata()?.len() + bytes.len() as u64 > MAX_LOG_BYTES {
            *file = None;
            std::fs::rename(&self.path, self.path.with_extension("log.1"))?;
            *file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        file.as_mut().expect("opened above").write_all(bytes)
    }
}

/// One line at a time into the rotating file.
pub struct LineWriter<'a>(&'a RotatingFile);

impl Write for LineWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write_line(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> fmt::MakeWriter<'a> for RotatingFile {
    type Writer = LineWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LineWriter(self)
    }
}

/// Starts logging to `dir`'s connection log, and to stderr when `YTTT_LOG`
/// is set. Once per process; later calls do nothing.
pub fn start(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let file = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(RotatingFile::new(dir))
        .with_filter(EnvFilter::new(FILE_FILTER));
    let stderr = std::env::var("YTTT_LOG")
        .ok()
        .and_then(|filter| EnvFilter::try_new(filter).ok())
        .map(|filter| {
            fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .with_filter(filter)
        });
    let _ = tracing_subscriber::registry()
        .with(file)
        .with(stderr)
        .try_init();
}

/// The end of the connection log, older file first, for the settings page.
pub fn read(dir: &Path) -> String {
    let mut bytes = std::fs::read(dir.join(LOG_FILE).with_extension("log.1")).unwrap_or_default();
    bytes.extend(std::fs::read(dir.join(LOG_FILE)).unwrap_or_default());
    let start = bytes.len().saturating_sub(READ_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_rotates_and_reads_back_from_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let file = RotatingFile::new(dir.path());
        let line = vec![b'x'; 1000];
        for _ in 0..1500 {
            let mut writer = LineWriter(&file);
            writer.write_all(&line).unwrap();
            writer.write_all(b"\n").unwrap();
        }
        let current = std::fs::metadata(dir.path().join(LOG_FILE)).unwrap().len();
        let older = std::fs::metadata(dir.path().join("connection.log.1"))
            .unwrap()
            .len();
        assert!(current <= MAX_LOG_BYTES && older <= MAX_LOG_BYTES);
        LineWriter(&file).write_all(b"the last line\n").unwrap();
        let tail = read(dir.path());
        assert!(tail.len() <= READ_BYTES);
        assert!(tail.ends_with("the last line\n"));
        assert_eq!(read(tempfile::tempdir().unwrap().path()), "");
    }
}
