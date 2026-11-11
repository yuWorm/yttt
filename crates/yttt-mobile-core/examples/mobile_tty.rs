//! Spike: runs the mobile core link in a desktop terminal, so the whole Rust
//! path (pairing, P2P session, observer attach, VT repaint, control) can be
//! checked before any phone app exists.
//!
//! ```text
//! cargo run -p yttt-mobile-core --example mobile_tty -- pair 'YTTT-PAIR:1:…'
//! cargo run -p yttt-mobile-core --example mobile_tty -- list
//! cargo run -p yttt-mobile-core --example mobile_tty -- watch <terminal> [--control]
//! ```
//!
//! State (paired Host record and device secret) lives in
//! `target/mobile-tty-state/`. Spike only: the apps keep the secret in the
//! Keychain or Keystore. Press Ctrl-] to quit `watch`.

use std::{
    io::{Read as _, Write as _},
    path::PathBuf,
    time::Duration,
};

use yttt_mobile_core::session::{MobileSession, PairedHostRecord, RelayChoice, pair};

fn state_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/mobile-tty-state")
}

fn load() -> Result<(PairedHostRecord, [u8; 32]), String> {
    let dir = state_dir();
    let record = std::fs::read(dir.join("host.json"))
        .map_err(|error| format!("not paired yet ({error}); run `pair` first"))?;
    let secret = std::fs::read(dir.join("device-secret")).map_err(|error| error.to_string())?;
    let secret: [u8; 32] = secret
        .try_into()
        .map_err(|_| "device secret is not 32 bytes".to_string())?;
    Ok((
        PairedHostRecord::from_bytes(&record).map_err(|error| error.to_string())?,
        secret,
    ))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), String> {
    // `YTTT_LOG=yttt=debug,iroh=info` logs to stderr.
    if let Ok(filter) = std::env::var("YTTT_LOG") {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .with_writer(std::io::stderr)
            .init();
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pair") => {
            let code = args.get(1).ok_or("usage: pair <YTTT-PAIR:1:…>")?;
            let secret: [u8; 32] = rand::random();
            let record = pair(
                code,
                "mobile_tty",
                secret,
                RelayChoice::FromHost,
                |short_code| {
                    println!("Compare this code with the Host and confirm there: {short_code:06}");
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            let dir = state_dir();
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            std::fs::write(dir.join("host.json"), record.to_bytes()).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("device-secret"), secret).map_err(|e| e.to_string())?;
            println!("Paired with {}.", record.host.host_label);
            Ok(())
        }
        Some("list") => {
            let (record, secret) = load()?;
            let session = connect(&record, secret).await?;
            for terminal in session.terminals().await.map_err(|e| e.to_string())? {
                println!(
                    "{}  ({}×{}, project {})",
                    terminal.session_id, terminal.cols, terminal.rows, terminal.project_id
                );
            }
            session.close().await;
            Ok(())
        }
        Some("watch") => {
            let id = args
                .get(1)
                .ok_or("usage: watch <terminal> [--control]")?
                .clone();
            let control = args.iter().any(|arg| arg == "--control");
            let (record, secret) = load()?;
            let session = connect(&record, secret).await?;
            watch(session, id, control).await
        }
        _ => Err("usage: mobile_tty pair <code> | list | watch <terminal> [--control]".into()),
    }
}

async fn connect(record: &PairedHostRecord, secret: [u8; 32]) -> Result<MobileSession, String> {
    MobileSession::connect(record, secret, RelayChoice::FromHost)
        .await
        .map_err(|error| error.to_string())
}

async fn watch(session: MobileSession, id: String, control: bool) -> Result<(), String> {
    let host_size = session
        .terminals()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|terminal| terminal.session_id == id)
        .map(|terminal| (terminal.cols, terminal.rows))
        .ok_or_else(|| format!("no terminal {id}"))?;
    session
        .attach(&id)
        .await
        .map_err(|error| error.to_string())?;
    let raw = RawMode::enable();
    let mut stdout = std::io::stdout();
    // The alternate screen keeps the shell's scrollback clean.
    let _ = stdout.write_all(b"\x1b[?1049h");
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0u8; 1024];
        while let Ok(read) = stdin.read(&mut buffer) {
            if read == 0 || input_tx.send(buffer[..read].to_vec()).is_err() {
                return;
            }
        }
    });
    if control {
        session
            .take_control()
            .await
            .map_err(|error| error.to_string())?;
        if let Some((cols, rows)) = local_size() {
            session
                .resize(&id, cols, rows)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    let mut frame = tokio::time::interval(Duration::from_millis(16));
    let quit = loop {
        tokio::select! {
            _ = frame.tick() => {
                if let Some(bytes) = session.drain(&id) {
                    let _ = stdout.write_all(&bytes);
                    let _ = stdout.flush();
                }
            }
            input = input_rx.recv() => {
                let Some(input) = input else { break None };
                if input.contains(&0x1d) {
                    break None;
                }
                if session.in_control(&id)
                    && let Err(error) = session.send_input(&id, input) {
                        break Some(error.to_string());
                    }
            }
        }
    };
    let _ = stdout.write_all(b"\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l");
    let _ = stdout.flush();
    drop(raw);
    let stats = session.drain_stats();
    println!(
        "Host size {}×{}. drain: {} frames, {} bytes, p50 {} µs, p95 {} µs, max {} µs",
        host_size.0,
        host_size.1,
        stats.frames,
        stats.bytes,
        stats.p50_micros,
        stats.p95_micros,
        stats.max_micros
    );
    session.close().await;
    quit.map_or(Ok(()), Err)
}

fn local_size() -> Option<(u16, u16)> {
    // SAFETY: TIOCGWINSZ fills the zeroed `winsize` for a terminal fd.
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        (libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0)
            .then_some((size.ws_col, size.ws_row))
    }
}

/// Puts the local terminal in raw mode while alive.
struct RawMode(Option<libc::termios>);

impl RawMode {
    fn enable() -> Self {
        // SAFETY: tcgetattr/tcsetattr on stdin with a zeroed termios.
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                return Self(None);
            }
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            Self(Some(original))
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(original) = self.0 {
            // SAFETY: restores the attributes read in `enable`.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &original);
            }
        }
    }
}
