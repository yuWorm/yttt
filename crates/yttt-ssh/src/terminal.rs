use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::watch;

pub(crate) const TERMINAL_CHUNK_BYTES: usize = 64 * 1024;
const TERMINAL_QUEUE_CAPACITY: usize = 32;
use yttt_core::model::{ids::ConnectionId, project::RemotePathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteTerminalExecution {
    Shell { command: String },
    Command { program: String, args: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTerminalRequest {
    pub connection_id: ConnectionId,
    pub cwd: RemotePathBuf,
    pub execution: RemoteTerminalExecution,
    pub environment: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCommandRequest {
    pub cwd: RemotePathBuf,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: Option<i32>,
}

impl RemoteCommandOutput {
    pub fn success(&self) -> bool {
        self.exit_status == Some(0)
    }
}

pub struct RemoteTerminalIo {
    pub writer: RemoteTerminalWriter,
    pub reader: RemoteTerminalReader,
}

pub struct RemoteTerminalSession {
    io: Option<RemoteTerminalIo>,
    commands: flume::Sender<RemoteTerminalCommand>,
    state: Arc<Mutex<RemoteTerminalState>>,
    closed: bool,
    shutdown: watch::Sender<bool>,
}

#[derive(Clone)]
pub struct RemoteTerminalResizeHandle {
    commands: flume::Sender<RemoteTerminalCommand>,
}

pub struct RemoteTerminalWriter {
    commands: flume::Sender<RemoteTerminalCommand>,
}

pub struct RemoteTerminalReader {
    output: flume::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
}

#[derive(Debug)]
pub(crate) enum RemoteTerminalCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Default)]
pub(crate) struct RemoteTerminalState {
    pub exit_code: Option<i32>,
    pub finished: bool,
}

pub(crate) struct RemoteTerminalEndpoint {
    pub shutdown: watch::Receiver<bool>,
    pub commands: flume::Receiver<RemoteTerminalCommand>,
    pub output: flume::Sender<Vec<u8>>,
    pub state: Arc<Mutex<RemoteTerminalState>>,
}

impl RemoteTerminalSession {
    pub(crate) fn channel() -> (Self, RemoteTerminalEndpoint) {
        let (commands_tx, commands_rx) = flume::bounded(TERMINAL_QUEUE_CAPACITY);
        let (output_tx, output_rx) = flume::bounded(TERMINAL_QUEUE_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(RemoteTerminalState::default()));
        (
            Self {
                io: Some(RemoteTerminalIo {
                    writer: RemoteTerminalWriter {
                        commands: commands_tx.clone(),
                    },
                    reader: RemoteTerminalReader {
                        output: output_rx,
                        pending: Vec::new(),
                        offset: 0,
                    },
                }),
                commands: commands_tx,
                state: state.clone(),
                closed: false,
                shutdown,
            },
            RemoteTerminalEndpoint {
                shutdown: shutdown_rx,
                commands: commands_rx,
                output: output_tx,
                state,
            },
        )
    }

    pub fn take_io(&mut self) -> Option<RemoteTerminalIo> {
        self.io.take()
    }

    pub fn resize_handle(&self) -> RemoteTerminalResizeHandle {
        RemoteTerminalResizeHandle {
            commands: self.commands.clone(),
        }
    }

    pub fn finish(mut self, terminate: bool) -> Option<i32> {
        if terminate {
            self.shutdown.send_replace(true);
        }
        self.closed = true;
        self.state.lock().ok().and_then(|state| state.exit_code)
    }
}

impl Drop for RemoteTerminalSession {
    fn drop(&mut self) {
        if !self.closed {
            self.shutdown.send_replace(true);
        }
    }
}

impl RemoteTerminalResizeHandle {
    pub fn resize(&self, cols: usize, rows: usize) -> Result<(), String> {
        let cols =
            u16::try_from(cols).map_err(|_| "terminal column count exceeds u16".to_string())?;
        let rows = u16::try_from(rows).map_err(|_| "terminal row count exceeds u16".to_string())?;
        self.commands
            .try_send(RemoteTerminalCommand::Resize { cols, rows })
            .map_err(|error| format!("remote terminal resize was not queued: {error}"))
    }
}

impl Write for RemoteTerminalWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let count = buffer.len().min(TERMINAL_CHUNK_BYTES);
        self.commands
            .send_timeout(
                RemoteTerminalCommand::Write(buffer[..count].to_vec()),
                Duration::from_secs(30),
            )
            .map_err(|error| match error {
                flume::SendTimeoutError::Timeout(_) => io::Error::new(
                    io::ErrorKind::TimedOut,
                    "remote terminal input is backpressured",
                ),
                flume::SendTimeoutError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "remote terminal is closed")
                }
            })?;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for RemoteTerminalReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.offset == self.pending.len() {
            self.pending = match self.output.recv() {
                Ok(output) => output,
                Err(_) => return Ok(0),
            };
            self.offset = 0;
        }
        let available = &self.pending[self.offset..];
        let len = available.len().min(buffer.len());
        buffer[..len].copy_from_slice(&available[..len]);
        self.offset += len;
        Ok(len)
    }
}

pub(crate) fn terminal_error_output(message: &str) -> Vec<u8> {
    format!("\r\n[yttt SSH] {message}\r\n").into_bytes()
}

pub(crate) fn finish_remote_terminal(
    state: &Arc<Mutex<RemoteTerminalState>>,
    exit_code: Option<i32>,
) {
    if let Ok(mut state) = state.lock() {
        state.exit_code = exit_code;
        state.finished = true;
    }
}

pub(crate) fn remote_shell_startup(
    cwd: &RemotePathBuf,
    command: &str,
    environment: &BTreeMap<String, String>,
) -> String {
    let mut startup = remote_environment_exports(environment);
    if !startup.is_empty() {
        startup.push_str(" && ");
    }
    startup.push_str("cd -- ");
    startup.push_str(&shell_quote(cwd.as_str()));
    if !command.trim().is_empty() {
        startup.push_str(" && ");
        startup.push_str(command);
    }
    startup.push('\r');
    startup
}

pub(crate) fn remote_exec_command(cwd: &RemotePathBuf, program: &str, args: &[String]) -> String {
    remote_exec_command_with_environment(cwd, program, args, &BTreeMap::new())
}

pub(crate) fn remote_exec_command_with_environment(
    cwd: &RemotePathBuf,
    program: &str,
    args: &[String],
    environment: &BTreeMap<String, String>,
) -> String {
    let mut command = remote_environment_exports(environment);
    if !command.is_empty() {
        command.push_str(" && ");
    }
    command.push_str("cd -- ");
    command.push_str(&shell_quote(cwd.as_str()));
    command.push_str(" && exec ");
    command.push_str(&shell_quote(program));
    for arg in args {
        command.push(' ');
        command.push_str(&shell_quote(arg));
    }
    command
}

fn remote_environment_exports(environment: &BTreeMap<String, String>) -> String {
    let mut exports = String::new();
    for (name, value) in environment {
        if !exports.is_empty() {
            exports.push_str(" && ");
        }
        let mut assignment = String::with_capacity(name.len() + value.len() + 1);
        assignment.push_str(name);
        assignment.push('=');
        assignment.push_str(value);
        exports.push_str("export ");
        exports.push_str(&shell_quote(&assignment));
    }
    exports
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_commands_quote_cwd_program_and_arguments() {
        let cwd = RemotePathBuf::new("/srv/team's app").unwrap();
        assert_eq!(
            remote_exec_command(&cwd, "printf", &["a b".to_string(), "x'y".to_string()]),
            "cd -- '/srv/team'\"'\"'s app' && exec 'printf' 'a b' 'x'\"'\"'y'"
        );
        assert_eq!(
            remote_shell_startup(&cwd, "cargo test", &BTreeMap::new()),
            "cd -- '/srv/team'\"'\"'s app' && cargo test\r"
        );
    }

    #[test]
    fn remote_shells_and_commands_export_configured_environment() {
        let cwd = RemotePathBuf::new("/srv/app").unwrap();
        let environment = BTreeMap::from([("YTTT_TOKEN".to_string(), "a b'c".to_string())]);

        assert_eq!(
            remote_shell_startup(&cwd, "", &environment),
            "export 'YTTT_TOKEN=a b'\"'\"'c' && cd -- '/srv/app'\r"
        );
        assert_eq!(
            remote_exec_command_with_environment(
                &cwd,
                "printenv",
                &["YTTT_TOKEN".to_string()],
                &environment,
            ),
            "export 'YTTT_TOKEN=a b'\"'\"'c' && cd -- '/srv/app' && exec 'printenv' 'YTTT_TOKEN'"
        );
    }

    #[test]
    fn remote_terminal_io_bridges_blocking_reader_and_writer() {
        let (mut session, endpoint) = RemoteTerminalSession::channel();
        let mut io = session.take_io().unwrap();
        io.writer.write_all(b"hello").unwrap();
        match endpoint.commands.try_recv().unwrap() {
            RemoteTerminalCommand::Write(bytes) => assert_eq!(bytes.as_slice(), b"hello"),
            command => panic!("unexpected command: {command:?}"),
        }
        endpoint.output.send(b"world".to_vec()).unwrap();
        drop(endpoint.output);
        let mut output = String::new();
        io.reader.read_to_string(&mut output).unwrap();
        assert_eq!(output, "world");
    }
}

#[cfg(test)]
mod backpressure_tests {
    use super::*;

    #[test]
    fn terminal_queues_are_bounded_and_shutdown_bypasses_backlog() {
        let (session, endpoint) = RemoteTerminalSession::channel();
        for _ in 0..TERMINAL_QUEUE_CAPACITY {
            session
                .commands
                .try_send(RemoteTerminalCommand::Write(vec![0; TERMINAL_CHUNK_BYTES]))
                .unwrap();
            endpoint
                .output
                .try_send(vec![0; TERMINAL_CHUNK_BYTES])
                .unwrap();
        }
        assert!(matches!(
            session
                .commands
                .try_send(RemoteTerminalCommand::Resize { cols: 80, rows: 24 }),
            Err(flume::TrySendError::Full(_))
        ));
        assert!(matches!(
            endpoint.output.try_send(vec![0]),
            Err(flume::TrySendError::Full(_))
        ));
        drop(session);
        assert!(*endpoint.shutdown.borrow());
    }

    #[test]
    fn large_terminal_writes_are_split_into_bounded_chunks() {
        let (mut session, endpoint) = RemoteTerminalSession::channel();
        let mut io = session.take_io().unwrap();
        let written = io.writer.write(&vec![0; TERMINAL_CHUNK_BYTES * 2]).unwrap();
        assert_eq!(written, TERMINAL_CHUNK_BYTES);
        assert!(
            matches!(endpoint.commands.try_recv().unwrap(), RemoteTerminalCommand::Write(bytes) if bytes.len() == written)
        );
    }
}
