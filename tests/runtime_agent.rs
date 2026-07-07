use yttt::runtime::terminal::{
    ExitReason, FakeTerminalRuntime, ProcessStatus, TerminalRuntime,
};

#[test]
fn fake_runtime_marks_process_running_then_exited() {
    let mut runtime = FakeTerminalRuntime::default();

    let pane = runtime.spawn("server", "echo ok").unwrap();
    assert_eq!(runtime.status(pane), Some(ProcessStatus::Running));

    runtime.exit(pane, 0, ExitReason::Completed);

    assert_eq!(
        runtime.status(pane),
        Some(ProcessStatus::Exited { code: Some(0) })
    );
}
