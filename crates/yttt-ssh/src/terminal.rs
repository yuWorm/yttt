use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex, mpsc as blocking_mpsc},
};

use tokio::sync::mpsc;
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
    commands: mpsc::UnboundedSender<RemoteTerminalCommand>,
    state: Arc<Mutex<RemoteTerminalState>>,
    closed: bool,
}

#[derive(Clone)]
pub struct RemoteTerminalResizeHandle {
    commands: mpsc::UnboundedSender<RemoteTerminalCommand>,
}

pub struct RemoteTerminalWriter {
    commands: mpsc::UnboundedSender<RemoteTerminalCommand>,
}

pub struct RemoteTerminalReader {
    output: blocking_mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
}

#[derive(Debug)]
pub(crate) enum RemoteTerminalCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

#[derive(Debug, Default)]
pub(crate) struct RemoteTerminalState {
    pub exit_code: Option<i32>,
    pub finished: bool,
}

pub(crate) struct RemoteTerminalEndpoint {
    pub commands: mpsc::UnboundedReceiver<RemoteTerminalCommand>,
    pub output: blocking_mpsc::Sender<Vec<u8>>,
    pub state: Arc<Mutex<RemoteTerminalState>>,
}

impl RemoteTerminalSession {
    pub(crate) fn channel() -> (Self, RemoteTerminalEndpoint) {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (output_tx, output_rx) = blocking_mpsc::channel();
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
            },
            RemoteTerminalEndpoint {
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
            let _ = self.commands.send(RemoteTerminalCommand::Shutdown);
        }
        self.closed = true;
        self.state.lock().ok().and_then(|state| state.exit_code)
    }
}

impl Drop for RemoteTerminalSession {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.commands.send(RemoteTerminalCommand::Shutdown);
        }
    }
}

impl RemoteTerminalResizeHandle {
    pub fn resize(&self, cols: usize, rows: usize) -> Result<(), String> {
        let cols =
            u16::try_from(cols).map_err(|_| "terminal column count exceeds u16".to_string())?;
        let rows = u16::try_from(rows).map_err(|_| "terminal row count exceeds u16".to_string())?;
        self.commands
            .send(RemoteTerminalCommand::Resize { cols, rows })
            .map_err(|_| "remote terminal is closed".to_string())
    }
}

impl Write for RemoteTerminalWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.commands
            .send(RemoteTerminalCommand::Write(buffer.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "remote terminal is closed"))?;
        Ok(buffer.len())
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

pub(crate) fn remote_shell_startup(cwd: &RemotePathBuf, command: &str) -> String {
    let cwd = shell_quote(cwd.as_str());
    if command.trim().is_empty() {
        format!("cd -- {cwd}\r")
    } else {
        format!("cd -- {cwd} && {command}\r")
    }
}

pub(crate) fn remote_exec_command(cwd: &RemotePathBuf, program: &str, args: &[String]) -> String {
    let mut command = format!(
        "cd -- {} && exec {}",
        shell_quote(cwd.as_str()),
        shell_quote(program)
    );
    for arg in args {
        command.push(' ');
        command.push_str(&shell_quote(arg));
    }
    command
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
            remote_shell_startup(&cwd, "cargo test"),
            "cd -- '/srv/team'\"'\"'s app' && cargo test\r"
        );
    }

    #[test]
    fn remote_terminal_io_bridges_blocking_reader_and_writer() {
        let (mut session, mut endpoint) = RemoteTerminalSession::channel();
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
