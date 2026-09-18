use std::time::Duration;

use yttt_terminal::{PortablePtyRuntime, ProcessStatus, TerminalRuntime, TerminalSpawnRequest};

#[test]
#[ignore = "spawns a real PTY process"]
fn real_runtime_runs_short_command_to_exit() {
    let mut runtime = PortablePtyRuntime::default();
    let handle = runtime
        .spawn(TerminalSpawnRequest::for_command(
            "probe",
            "sh",
            "printf",
            vec!["ok".to_string()],
        ))
        .unwrap();

    runtime
        .wait_for_exit(handle, Duration::from_secs(2))
        .unwrap();

    assert!(matches!(
        runtime.status(handle),
        Some(ProcessStatus::Exited { code: Some(0) })
    ));
}
