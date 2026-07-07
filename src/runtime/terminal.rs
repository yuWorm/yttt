use std::collections::HashMap;

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
