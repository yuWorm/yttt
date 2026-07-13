use yttt_terminal::pty::{
    ExitReason, FakeTerminalRuntime, PortablePtyRuntime, ProcessStatus, TerminalExecution,
    TerminalRuntime, TerminalSpawnRequest, spawn_portable_pty_session,
};

#[test]
fn fake_runtime_records_spawn_cwd_and_exit_status() {
    let mut runtime = FakeTerminalRuntime::default();
    let request = TerminalSpawnRequest::for_shell("server", "sh", "pwd").cwd("/tmp/yttt");

    let handle = runtime.spawn(request).unwrap();

    assert_eq!(
        runtime.spawn_cwd(handle).unwrap(),
        std::path::Path::new("/tmp/yttt")
    );
    assert_eq!(runtime.status(handle), Some(ProcessStatus::Running));

    runtime.exit(handle, 0, ExitReason::Completed);

    assert_eq!(
        runtime.status(handle),
        Some(ProcessStatus::Exited { code: Some(0) })
    );
}

#[test]
fn spawn_request_records_size_and_working_directory() {
    let request = TerminalSpawnRequest::for_shell("pane", "sh", "echo ok")
        .cwd("/tmp/yttt")
        .size(120, 32);

    assert_eq!(request.pane_id, "pane");
    assert_eq!(
        request.execution,
        TerminalExecution::Shell {
            shell: "sh".to_string(),
            command: "echo ok".to_string(),
        }
    );
    assert_eq!(request.cwd, std::path::PathBuf::from("/tmp/yttt"));
    assert_eq!(request.cols, 120);
    assert_eq!(request.rows, 32);
}

#[test]
fn direct_command_request_preserves_program_and_argument_boundaries() {
    let request = TerminalSpawnRequest::for_command(
        "pane",
        "zsh",
        "npm",
        vec!["run".to_string(), "dev server".to_string()],
    );

    assert_eq!(
        request.execution,
        TerminalExecution::Command {
            shell: "zsh".to_string(),
            program: "npm".to_string(),
            args: vec!["run".to_string(), "dev server".to_string()],
        }
    );
}

#[cfg(unix)]
#[test]
fn command_mode_exposes_program_output_through_the_pty() {
    use std::io::Read as _;

    let mut session = spawn_portable_pty_session(TerminalSpawnRequest::for_command(
        "probe",
        "/bin/sh",
        "printf",
        vec!["rendered-through-pty".to_string()],
    ))
    .unwrap();
    let mut io = session.take_io().unwrap();
    let mut output = String::new();

    io.reader.read_to_string(&mut output).unwrap();
    let status = session.finish(ExitReason::Completed).unwrap();

    assert_eq!(status, ProcessStatus::Exited { code: Some(0) });
    assert!(
        output.contains("rendered-through-pty"),
        "PTY output did not contain the command output: {output:?}"
    );
}

#[test]
#[ignore = "spawns a real PTY process"]
fn real_runtime_runs_short_command_to_exit() {
    let mut runtime = PortablePtyRuntime::default();
    let handle = runtime
        .spawn(TerminalSpawnRequest::for_shell("probe", "sh", "printf ok"))
        .unwrap();

    runtime
        .wait_for_exit(handle, std::time::Duration::from_secs(2))
        .unwrap();

    assert!(matches!(
        runtime.status(handle),
        Some(ProcessStatus::Exited { code: Some(0) })
    ));
}

#[test]
#[ignore = "spawns a real PTY process"]
fn real_session_exposes_io_and_resize_handle() {
    let mut session =
        spawn_portable_pty_session(TerminalSpawnRequest::for_shell("probe", "sh", "printf ok"))
            .unwrap();

    let io = session.take_io().unwrap();
    session.resize(100, 30).unwrap();
    drop(io);
    session.finish(ExitReason::KilledByUser).unwrap();
}
