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
        .envs([("YTTT_TEST", "injected")])
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
    assert_eq!(
        request.environment.get("YTTT_TEST"),
        Some(&"injected".to_string())
    );
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

#[cfg(unix)]
#[test]
fn command_mode_injects_configured_environment_variables() {
    use std::io::Read as _;

    let request = TerminalSpawnRequest::for_command(
        "probe",
        "/bin/sh",
        "/bin/sh",
        vec![
            "-c".to_string(),
            "printf '%s' \"$YTTT_INJECTED_COMMAND\"".to_string(),
        ],
    )
    .envs([("YTTT_INJECTED_COMMAND", "command value")]);
    let mut session = spawn_portable_pty_session(request).unwrap();
    let mut io = session.take_io().unwrap();
    let mut output = String::new();

    io.reader.read_to_string(&mut output).unwrap();
    let status = session.finish(ExitReason::Completed).unwrap();

    assert_eq!(status, ProcessStatus::Exited { code: Some(0) });
    assert!(
        output.contains("command value"),
        "PTY output did not contain the injected value: {output:?}"
    );
}

#[cfg(unix)]
#[test]
fn shell_mode_injects_configured_environment_variables() {
    use std::io::Read as _;

    let request = TerminalSpawnRequest::for_shell(
        "probe",
        "/bin/sh",
        "printf '%s' \"$YTTT_INJECTED_SHELL\"; exit",
    )
    .envs([("YTTT_INJECTED_SHELL", "shell value")]);
    let mut session = spawn_portable_pty_session(request).unwrap();
    let mut io = session.take_io().unwrap();
    let mut output = String::new();

    io.reader.read_to_string(&mut output).unwrap();
    let status = session.finish(ExitReason::Completed).unwrap();

    assert_eq!(status, ProcessStatus::Exited { code: Some(0) });
    assert!(
        output.contains("shell value"),
        "PTY output did not contain the injected value: {output:?}"
    );
}

#[cfg(unix)]
#[test]
fn shell_mode_returns_to_the_shell_after_the_initial_command() {
    use std::io::{Read as _, Write as _};

    let mut session =
        spawn_portable_pty_session(TerminalSpawnRequest::for_shell("probe", "/bin/sh", "false"))
            .unwrap();
    let mut io = session.take_io().unwrap();

    io.writer.write_all(b"true\nexit\n").unwrap();
    io.writer.flush().unwrap();
    let mut output = String::new();
    io.reader.read_to_string(&mut output).unwrap();
    let status = session.finish(ExitReason::Completed).unwrap();

    assert_eq!(status, ProcessStatus::Exited { code: Some(0) });
}

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
        .wait_for_exit(handle, std::time::Duration::from_secs(2))
        .unwrap();

    assert!(matches!(
        runtime.status(handle),
        Some(ProcessStatus::Exited { code: Some(0) })
    ));
}

#[cfg(unix)]
#[test]
#[ignore = "spawns a real PTY and requires python3"]
fn resized_pty_reports_cell_and_pixel_dimensions_to_its_process() {
    use std::io::{Read, Write};
    use std::time::Duration;

    let command = "import fcntl,struct,termios; input(); print('WINSIZE',*struct.unpack('HHHH',fcntl.ioctl(0,termios.TIOCGWINSZ,b'\\0'*8)),flush=True)";
    let mut session = spawn_portable_pty_session(TerminalSpawnRequest::for_command(
        "geometry",
        "sh",
        "python3",
        vec!["-c".into(), command.into()],
    ))
    .unwrap();
    let mut io = session.take_io().unwrap();
    session.resize(100, 30, 9, 18).unwrap();
    io.writer.write_all(b"\n").unwrap();
    io.writer.flush().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            match io.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => output.extend_from_slice(&buffer[..length]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
            if output
                .windows(24)
                .any(|bytes| bytes == b"WINSIZE 30 100 900 540\r\n")
            {
                break;
            }
        }
        let _ = tx.send(output);
    });
    let output = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        String::from_utf8_lossy(&output).contains("WINSIZE 30 100 900 540"),
        "{output:?}"
    );
    session.finish(ExitReason::Completed).unwrap();
}
