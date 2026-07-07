use std::time::Duration;

use yttt::runtime::terminal::{
    PortablePtyRuntime, ProcessStatus, TerminalRuntime, spawn_portable_pty_session,
};

#[test]
#[ignore = "spawns a real PTY process"]
fn real_runtime_runs_short_command_to_exit() {
    let mut runtime = PortablePtyRuntime::default();
    let handle = runtime.spawn("probe", "printf ok").unwrap();

    runtime
        .wait_for_exit(handle, Duration::from_secs(2))
        .unwrap();

    assert!(matches!(
        runtime.status(handle),
        Some(ProcessStatus::Exited { code: Some(0) })
    ));
}

#[test]
#[ignore = "spawns a real PTY process"]
fn real_session_exposes_io_and_resize_handle() {
    let mut session = spawn_portable_pty_session("probe", "printf ok", 80, 24).unwrap();

    let io = session.take_io().unwrap();
    session.resize(100, 30).unwrap();
    drop(io);
    session.kill().unwrap();
}
