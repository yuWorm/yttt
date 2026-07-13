mod driver;

pub use driver::{
    MAX_COMMANDS, MAX_QUEUED_INPUT_BYTES, MAX_QUEUED_REPLY_BYTES, MAX_USER_COMMANDS,
    MAX_WRITE_CHUNK_BYTES, PtyEvent, PtyIoOperation, READ_BUFFER_BYTES, READ_QUEUE_CAPACITY,
    ResizeCallback,
};
pub(crate) use driver::{PtyIoDriver, PtyIoHandle};

use std::collections::HashMap;
use std::io::{self, Read, Write};
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
    Shell {
        shell: String,
        command: String,
    },
    Command {
        shell: String,
        program: String,
        args: Vec<String>,
    },
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
        shell: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            pane_id: pane_id.into(),
            execution: TerminalExecution::Command {
                shell: shell.into(),
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
    reaped: bool,
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
        reaped: false,
    })
}

const COMPLETED_EXIT_GRACE: Duration = Duration::from_millis(500);

trait ReapableChild {
    fn poll_exit(&mut self) -> io::Result<Option<portable_pty::ExitStatus>>;
    fn terminate(&mut self) -> io::Result<()>;
    fn wait_for_exit(&mut self) -> io::Result<portable_pty::ExitStatus>;
}

impl<T: Child + ?Sized> ReapableChild for T {
    fn poll_exit(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
        self.try_wait()
    }

    fn terminate(&mut self) -> io::Result<()> {
        self.kill()
    }

    fn wait_for_exit(&mut self) -> io::Result<portable_pty::ExitStatus> {
        self.wait()
    }
}

fn reap_child<C: ReapableChild + ?Sized>(
    child: &mut C,
    reason: ExitReason,
    completed_grace: Duration,
) -> io::Result<portable_pty::ExitStatus> {
    let mut exited = false;
    if reason == ExitReason::Completed {
        let deadline = Instant::now() + completed_grace;
        loop {
            match child.poll_exit() {
                Ok(Some(_)) => {
                    exited = true;
                    break;
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) | Err(_) => break,
            }
        }
    }

    if reason != ExitReason::Completed || !exited {
        let _ = child.terminate();
    }
    child.wait_for_exit()
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

    /// Consume the session and reap its child exactly once.
    pub fn finish(mut self, reason: ExitReason) -> anyhow::Result<ProcessStatus> {
        if self.reaped {
            return Ok(self.status);
        }
        let result = reap_child(&mut *self.child, reason, COMPLETED_EXIT_GRACE);
        self.reaped = true;
        let status = result?;
        self.status = ProcessStatus::Exited {
            code: Some(exit_status_code(status)),
        };
        Ok(self.status)
    }
}
impl Drop for PortablePtySession {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        let _ = reap_child(&mut *self.child, ExitReason::KilledByUser, Duration::ZERO);
        self.reaped = true;
        debug_assert!(
            false,
            "PortablePtySession dropped without consuming finish/reap"
        );
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
        TerminalExecution::Command {
            shell,
            program,
            args,
        } => {
            let shell = shell.trim();
            if shell.is_empty() {
                anyhow::bail!("command shell cannot be empty");
            }
            let program = program.trim();
            if program.is_empty() {
                anyhow::bail!("command executable cannot be empty");
            }

            command_execution_builder(shell, program, args)
        }
    };
    configure_terminal_environment(&mut builder);
    builder.cwd(cwd);
    Ok(builder)
}

/// Configure a PTY child process with yttt's terminal capabilities.
///
/// Existing `CLICOLOR` preferences are preserved, and the default is omitted
/// when the parent environment explicitly requests `NO_COLOR`.
pub fn configure_terminal_environment(builder: &mut CommandBuilder) {
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "yttt");
    builder.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));

    if builder.get_env("CLICOLOR").is_none() && builder.get_env("NO_COLOR").is_none() {
        builder.env("CLICOLOR", "1");
    }
}

fn command_execution_builder(shell: &str, program: &str, args: &[String]) -> CommandBuilder {
    #[cfg(unix)]
    {
        let shell_name = shell_executable_name(shell);
        if matches!(shell_name.as_str(), "fish" | "fish.exe") {
            let mut builder = CommandBuilder::new(shell);
            builder.args(["-lic", "exec $argv", "--", program]);
            builder.args(args);
            return builder;
        }
        if matches!(
            shell_name.as_str(),
            "sh" | "sh.exe"
                | "ash"
                | "ash.exe"
                | "bash"
                | "bash.exe"
                | "dash"
                | "dash.exe"
                | "ksh"
                | "ksh.exe"
                | "mksh"
                | "mksh.exe"
                | "zsh"
                | "zsh.exe"
        ) {
            let mut builder = CommandBuilder::new(shell);
            builder.args(["-lic", "exec \"$@\"", "yttt-command", program]);
            builder.args(args);
            return builder;
        }
    }

    let mut builder = CommandBuilder::new(program);
    builder.args(args);
    builder
}

fn shell_executable_name(shell: &str) -> String {
    shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase()
}

fn shell_execution_args(shell: &str, command: &str) -> Vec<String> {
    let shell_name = shell_executable_name(shell);

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
        "sh" | "sh.exe"
            | "ash"
            | "ash.exe"
            | "bash"
            | "bash.exe"
            | "dash"
            | "dash.exe"
            | "fish"
            | "fish.exe"
            | "ksh"
            | "ksh.exe"
            | "mksh"
            | "mksh.exe"
            | "zsh"
            | "zsh.exe"
    ) {
        vec!["-lic".to_string(), command.to_string()]
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
    fn command_execution_preserves_arguments_and_uses_the_user_shell_environment() {
        let execution = TerminalExecution::Command {
            shell: "/bin/zsh".to_string(),
            program: "npm".to_string(),
            args: vec!["run".to_string(), "dev server".to_string()],
        };

        let expected = if cfg!(unix) {
            vec![
                "/bin/zsh",
                "-lic",
                "exec \"$@\"",
                "yttt-command",
                "npm",
                "run",
                "dev server",
            ]
        } else {
            vec!["npm", "run", "dev server"]
        };
        assert_eq!(
            argv(command_builder(&execution, Path::new("/tmp")).unwrap()),
            expected
        );
    }
    use crate::test_support::FakeReaperState;

    struct FakeReapChild {
        state: FakeReaperState,
        exited_on_poll: bool,
    }

    impl ReapableChild for FakeReapChild {
        fn poll_exit(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
            Ok(self
                .exited_on_poll
                .then(|| portable_pty::ExitStatus::with_exit_code(0)))
        }

        fn terminate(&mut self) -> io::Result<()> {
            self.state.record_kill();
            Ok(())
        }

        fn wait_for_exit(&mut self) -> io::Result<portable_pty::ExitStatus> {
            self.state.record_wait();
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }
    }

    #[test]
    fn portable_pty_reaper_kills_and_waits_exactly_once() {
        let state = FakeReaperState::default();
        let mut child = FakeReapChild {
            state: state.clone(),
            exited_on_poll: false,
        };
        reap_child(&mut child, ExitReason::Failed, Duration::ZERO).unwrap();
        assert_eq!(state.counts(), (1, 1));

        let exited_state = FakeReaperState::default();
        let mut exited_child = FakeReapChild {
            state: exited_state.clone(),
            exited_on_poll: true,
        };
        reap_child(&mut exited_child, ExitReason::Completed, Duration::ZERO).unwrap();
        assert_eq!(exited_state.counts(), (0, 1));

        let stalled_state = FakeReaperState::default();
        let mut stalled_child = FakeReapChild {
            state: stalled_state.clone(),
            exited_on_poll: false,
        };
        reap_child(&mut stalled_child, ExitReason::Completed, Duration::ZERO).unwrap();
        assert_eq!(stalled_state.counts(), (1, 1));
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
            vec!["/bin/sh", "-lic", "echo ok"]
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

    #[test]
    fn spawned_commands_advertise_terminal_color_capabilities() {
        let execution = TerminalExecution::Shell {
            shell: "/bin/zsh".to_string(),
            command: String::new(),
        };
        let builder = command_builder(&execution, Path::new("/tmp")).unwrap();

        assert_eq!(
            builder.get_env("TERM"),
            Some(std::ffi::OsStr::new("xterm-256color"))
        );
        assert_eq!(
            builder.get_env("COLORTERM"),
            Some(std::ffi::OsStr::new("truecolor"))
        );
        assert_eq!(
            builder.get_env("TERM_PROGRAM"),
            Some(std::ffi::OsStr::new("yttt"))
        );
        assert_eq!(
            builder.get_env("TERM_PROGRAM_VERSION"),
            Some(std::ffi::OsStr::new(env!("CARGO_PKG_VERSION")))
        );
    }

    #[test]
    fn spawned_commands_default_to_cli_colors_without_overriding_user_preferences() {
        let mut defaults = CommandBuilder::new("/bin/sh");
        defaults.env_remove("CLICOLOR");
        defaults.env_remove("NO_COLOR");
        configure_terminal_environment(&mut defaults);
        assert_eq!(
            defaults.get_env("CLICOLOR"),
            Some(std::ffi::OsStr::new("1"))
        );

        let mut explicit = CommandBuilder::new("/bin/sh");
        explicit.env("CLICOLOR", "custom");
        configure_terminal_environment(&mut explicit);
        assert_eq!(
            explicit.get_env("CLICOLOR"),
            Some(std::ffi::OsStr::new("custom"))
        );

        let mut disabled = CommandBuilder::new("/bin/sh");
        disabled.env_remove("CLICOLOR");
        disabled.env("NO_COLOR", "1");
        configure_terminal_environment(&mut disabled);
        assert_eq!(disabled.get_env("CLICOLOR"), None);
    }

    fn argv(builder: CommandBuilder) -> Vec<String> {
        builder
            .get_argv()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}
