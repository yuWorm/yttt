use std::collections::HashMap;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

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

pub trait TerminalRuntime {
    fn spawn(&mut self, pane_id: &str, command: &str) -> anyhow::Result<ProcessHandle>;
    fn kill(&mut self, handle: ProcessHandle) -> anyhow::Result<()>;
    fn status(&self, handle: ProcessHandle) -> Option<ProcessStatus>;
}

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
    fn spawn(&mut self, pane_id: &str, command: &str) -> anyhow::Result<ProcessHandle> {
        self.next_handle += 1;
        let handle = ProcessHandle(self.next_handle);

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let portable_pty::PtyPair { slave, master } = pair;
        let command_builder = shell_command(command);
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
                _pane_id: pane_id.to_string(),
                _command: command.to_string(),
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
}

impl TerminalRuntime for FakeTerminalRuntime {
    fn spawn(&mut self, pane_id: &str, command: &str) -> anyhow::Result<ProcessHandle> {
        self.next_handle += 1;
        let handle = ProcessHandle(self.next_handle);
        self.processes.insert(
            handle,
            FakeProcess {
                _pane_id: pane_id.to_string(),
                _command: command.to_string(),
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

struct FakeProcess {
    _pane_id: String,
    _command: String,
    status: ProcessStatus,
}

struct PortablePtyProcess {
    _pane_id: String,
    _command: String,
    _master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    status: ProcessStatus,
}

fn shell_command(command: &str) -> CommandBuilder {
    #[cfg(windows)]
    {
        let mut builder = CommandBuilder::new("cmd");
        builder.arg("/C");
        builder.arg(command);
        builder
    }

    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
        let mut builder = CommandBuilder::new(shell);
        builder.arg("-lc");
        builder.arg(command);
        builder
    }
}

fn exit_status_code(status: portable_pty::ExitStatus) -> i32 {
    status.exit_code() as i32
}
