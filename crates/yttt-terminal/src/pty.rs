use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcessHandle(u64);

impl ProcessHandle {
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited { code: Option<i32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitReason {
    Completed,
    Failed,
    KilledByUser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalExecution {
    Shell { shell: String, command: String },
    Command { program: String, args: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSpawnRequest {
    pub pane_id: String,
    pub execution: TerminalExecution,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSpawnRequest {
    pub fn for_shell(
        pane_id: impl Into<String>,
        shell: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            pane_id: pane_id.into(),
            execution: TerminalExecution::Shell {
                shell: shell.into(),
                command: command.into(),
            },
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            cols: 80,
            rows: 24,
        }
    }

    pub fn for_command(
        pane_id: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            pane_id: pane_id.into(),
            execution: TerminalExecution::Command {
                program: program.into(),
                args,
            },
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            cols: 80,
            rows: 24,
        }
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols;
        self.rows = rows;
        self
    }
}

pub trait TerminalRuntime {
    fn spawn(&mut self, request: TerminalSpawnRequest) -> anyhow::Result<ProcessHandle>;
    fn kill(&mut self, handle: ProcessHandle) -> anyhow::Result<()>;
    fn status(&self, handle: ProcessHandle) -> Option<ProcessStatus>;
}

pub struct PortablePtyIo {
    pub writer: Box<dyn Write + Send>,
    pub reader: Box<dyn Read + Send>,
}

pub struct PortablePtySession {
    _pane_id: String,
    _execution: TerminalExecution,
    _cwd: PathBuf,
    master: PortablePtyMaster,
    child: Box<dyn Child + Send + Sync>,
    io: Option<PortablePtyIo>,
    status: ProcessStatus,
}

#[derive(Clone)]
pub struct PortablePtyResizeHandle {
    master: PortablePtyMaster,
}

type PortablePtyMaster = Arc<Mutex<Box<dyn MasterPty + Send>>>;

#[derive(Default)]
pub struct FakeTerminalRuntime {
    next_handle: u64,
    processes: HashMap<ProcessHandle, FakeProcess>,
}

#[derive(Default)]
pub struct PortablePtyRuntime {
    next_handle: u64,
    processes: HashMap<ProcessHandle, PortablePtyProcess>,
}

impl PortablePtyRuntime {
    pub fn wait_for_exit(
        &mut self,
        handle: ProcessHandle,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.poll_exit(handle);
            if matches!(
                self.status(handle),
                Some(ProcessStatus::Exited { .. }) | None
            ) {
                return Ok(());
            }

            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for process {}", handle.raw());
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn poll_exit(&mut self, handle: ProcessHandle) {
        let Some(process) = self.processes.get_mut(&handle) else {
            return;
        };
        if matches!(process.status, ProcessStatus::Exited { .. }) {
            return;
        }

        if let Ok(Some(status)) = process.child.try_wait() {
            process.status = ProcessStatus::Exited {
                code: Some(exit_status_code(status)),
            };
        }
    }
}

impl TerminalRuntime for PortablePtyRuntime {
    fn spawn(&mut self, request: TerminalSpawnRequest) -> anyhow::Result<ProcessHandle> {
        self.next_handle += 1;
        let handle = ProcessHandle(self.next_handle);

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: request.rows,
            cols: request.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let portable_pty::PtyPair { slave, master } = pair;
        let command_builder = command_builder(&request.execution, &request.cwd)?;
        let child = slave.spawn_command(command_builder)?;
        drop(slave);

        let writer = master.take_writer()?;
        if cfg!(target_os = "macos") {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(writer);

        self.processes.insert(
            handle,
            PortablePtyProcess {
                _pane_id: request.pane_id,
                _execution: request.execution,
                _cwd: request.cwd,
                _master: master,
                child,
                status: ProcessStatus::Running,
            },
        );

        Ok(handle)
    }

    fn kill(&mut self, handle: ProcessHandle) -> anyhow::Result<()> {
        if let Some(process) = self.processes.get_mut(&handle) {
            process.child.kill()?;
            process.status = ProcessStatus::Exited { code: None };
        }
        Ok(())
    }

    fn status(&self, handle: ProcessHandle) -> Option<ProcessStatus> {
        self.processes.get(&handle).map(|process| process.status)
    }
}

impl FakeTerminalRuntime {
    pub fn exit(&mut self, handle: ProcessHandle, code: i32, _reason: ExitReason) {
        if let Some(process) = self.processes.get_mut(&handle) {
            process.status = ProcessStatus::Exited { code: Some(code) };
        }
    }

    pub fn spawn_cwd(&self, handle: ProcessHandle) -> Option<&Path> {
        self.processes
            .get(&handle)
            .map(|process| process.cwd.as_path())
    }
}

impl TerminalRuntime for FakeTerminalRuntime {
    fn spawn(&mut self, request: TerminalSpawnRequest) -> anyhow::Result<ProcessHandle> {
        self.next_handle += 1;
        let handle = ProcessHandle(self.next_handle);
        self.processes.insert(
            handle,
            FakeProcess {
                _pane_id: request.pane_id,
                _execution: request.execution,
                cwd: request.cwd,
                status: ProcessStatus::Running,
            },
        );
        Ok(handle)
    }

    fn kill(&mut self, handle: ProcessHandle) -> anyhow::Result<()> {
        if let Some(process) = self.processes.get_mut(&handle) {
            process.status = ProcessStatus::Exited { code: None };
        }
        Ok(())
    }

    fn status(&self, handle: ProcessHandle) -> Option<ProcessStatus> {
        self.processes.get(&handle).map(|process| process.status)
    }
}

pub fn spawn_portable_pty_session(
    request: TerminalSpawnRequest,
) -> anyhow::Result<PortablePtySession> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: request.rows,
        cols: request.cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let portable_pty::PtyPair { slave, master } = pair;
    let command_builder = command_builder(&request.execution, &request.cwd)?;
    let child = slave.spawn_command(command_builder)?;
    drop(slave);

    let reader = master.try_clone_reader()?;
    let writer = master.take_writer()?;
    let master = Arc::new(Mutex::new(master));

    Ok(PortablePtySession {
        _pane_id: request.pane_id,
        _execution: request.execution,
        _cwd: request.cwd,
        master,
        child,
        io: Some(PortablePtyIo { writer, reader }),
        status: ProcessStatus::Running,
    })
}

impl PortablePtySession {
    pub fn take_io(&mut self) -> Option<PortablePtyIo> {
        self.io.take()
    }

    pub fn resize_handle(&self) -> PortablePtyResizeHandle {
        PortablePtyResizeHandle {
            master: self.master.clone(),
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.resize_handle().resize(cols as usize, rows as usize)
    }

    pub fn kill(&mut self) -> anyhow::Result<()> {
        if matches!(self.status, ProcessStatus::Running) {
            self.child.kill()?;
            self.status = ProcessStatus::Exited { code: None };
        }
        Ok(())
    }

    pub fn status(&mut self) -> ProcessStatus {
        if matches!(self.status, ProcessStatus::Running)
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.status = ProcessStatus::Exited {
                code: Some(exit_status_code(status)),
            };
        }

        self.status
    }
}

impl PortablePtyResizeHandle {
    pub fn resize(&self, cols: usize, rows: usize) -> anyhow::Result<()> {
        let master = self
            .master
            .lock()
            .map_err(|_| anyhow::anyhow!("pty master lock poisoned"))?;
        master.resize(PtySize {
            cols: cols as u16,
            rows: rows as u16,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

struct FakeProcess {
    _pane_id: String,
    _execution: TerminalExecution,
    cwd: PathBuf,
    status: ProcessStatus,
}

struct PortablePtyProcess {
    _pane_id: String,
    _execution: TerminalExecution,
    _cwd: PathBuf,
    _master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    status: ProcessStatus,
}

fn command_builder(execution: &TerminalExecution, cwd: &Path) -> anyhow::Result<CommandBuilder> {
    let mut builder = match execution {
        TerminalExecution::Shell { shell, command } => {
            let shell = shell.trim();
            if shell.is_empty() {
                anyhow::bail!("shell executable cannot be empty");
            }

            let mut builder = CommandBuilder::new(shell);
            if !command.trim().is_empty() {
                for arg in shell_execution_args(shell, command) {
                    builder.arg(arg);
                }
            }
            builder
        }
        TerminalExecution::Command { program, args } => {
            let program = program.trim();
            if program.is_empty() {
                anyhow::bail!("command executable cannot be empty");
            }

            let mut builder = CommandBuilder::new(program);
            builder.args(args);
            builder
        }
    };
    builder.cwd(cwd);
    Ok(builder)
}

fn shell_execution_args(shell: &str, command: &str) -> Vec<String> {
    let shell_name = shell
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();

    if matches!(shell_name.as_str(), "cmd" | "cmd.exe") {
        vec![
            "/D".to_string(),
            "/S".to_string(),
            "/C".to_string(),
            command.to_string(),
        ]
    } else if matches!(
        shell_name.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    ) {
        vec![
            "-NoLogo".to_string(),
            "-Command".to_string(),
            command.to_string(),
        ]
    } else if matches!(
        shell_name.as_str(),
        "bash" | "bash.exe" | "zsh" | "zsh.exe" | "fish" | "fish.exe"
    ) {
        vec!["-lc".to_string(), command.to_string()]
    } else {
        vec!["-c".to_string(), command.to_string()]
    }
}

fn exit_status_code(status: portable_pty::ExitStatus) -> i32 {
    status.exit_code() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_execution_preserves_program_and_argument_boundaries() {
        let execution = TerminalExecution::Command {
            program: "npm".to_string(),
            args: vec!["run".to_string(), "dev server".to_string()],
        };

        assert_eq!(
            argv(command_builder(&execution, Path::new("/tmp")).unwrap()),
            vec!["npm", "run", "dev server"]
        );
    }

    #[test]
    fn shell_execution_uses_shell_specific_command_flags() {
        let sh = TerminalExecution::Shell {
            shell: "/bin/sh".to_string(),
            command: "echo ok".to_string(),
        };
        let powershell = TerminalExecution::Shell {
            shell: "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe".to_string(),
            command: "Write-Output ok".to_string(),
        };

        assert_eq!(
            argv(command_builder(&sh, Path::new("/tmp")).unwrap()),
            vec!["/bin/sh", "-c", "echo ok"]
        );
        assert_eq!(
            argv(command_builder(&powershell, Path::new("/tmp")).unwrap()),
            vec![
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                "-NoLogo",
                "-Command",
                "Write-Output ok",
            ]
        );
    }

    #[test]
    fn empty_shell_command_starts_an_interactive_shell() {
        let execution = TerminalExecution::Shell {
            shell: "/bin/zsh".to_string(),
            command: String::new(),
        };

        assert_eq!(
            argv(command_builder(&execution, Path::new("/tmp")).unwrap()),
            vec!["/bin/zsh"]
        );
    }

    fn argv(builder: CommandBuilder) -> Vec<String> {
        builder
            .get_argv()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}
