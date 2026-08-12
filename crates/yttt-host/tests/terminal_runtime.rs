#![cfg(unix)]

use std::time::Duration;

use yttt_core::model::ids::{ClientInstanceId, PaneId, ProjectId, TabId, TerminalSessionId};
use yttt_host::{runtime::HostRuntime, terminal::HostTerminalEvent};
use yttt_protocol::terminal::{
    SemanticViewport, TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode,
    TerminalSpawnSpec, TerminalStreamUpdate,
};
fn spec(session: &str, command: &str) -> TerminalSpawnSpec {
    TerminalSpawnSpec {
        session_id: TerminalSessionId::new(session),
        project_id: ProjectId::new("project"),
        tab_id: TabId::new("tab"),
        pane_id: PaneId::new(session),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        execution: TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec!["-lc".to_string(), command.to_string()],
            return_to_shell: false,
        },
        geometry: TerminalGeometry {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        },
        geometry_epoch: 1,
        environment: Vec::new(),
        removed_environment: Vec::new(),
        scrollback_limit: 1_000,
    }
}

fn viewport_text(viewport: &SemanticViewport) -> String {
    viewport
        .rows
        .iter()
        .flat_map(|row| row.spans.iter().map(|span| span.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn wait_for_text(
    events: &mut tokio::sync::broadcast::Receiver<HostTerminalEvent>,
    expected: &str,
) -> TerminalStreamUpdate {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let HostTerminalEvent::Update { update, .. } = events.recv().await.unwrap() else {
                continue;
            };
            let text = match &update {
                TerminalStreamUpdate::Snapshot(viewport) => viewport_text(viewport),
                TerminalStreamUpdate::Delta(delta) => delta
                    .changed_rows
                    .iter()
                    .flat_map(|row| row.spans.iter().map(|span| span.text.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                TerminalStreamUpdate::ResyncRequired { .. }
                | TerminalStreamUpdate::RawTail { .. } => String::new(),
            };
            if text.contains(expected) {
                return update;
            }
        }
    })
    .await
    .expect("terminal output timeout")
}

#[tokio::test]
async fn host_owns_terminal_after_the_spawning_client_handle_is_dropped() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let terminal = runtime
        .spawn(spec("survives-client", "printf host-survived; sleep 30"))
        .unwrap();
    let session_epoch = terminal.session_epoch();
    drop(terminal);

    let update = wait_for_text(&mut events, "host-survived").await;
    assert_eq!(
        match update {
            TerminalStreamUpdate::Snapshot(viewport) => viewport.session_epoch,
            TerminalStreamUpdate::Delta(delta) => delta.session_epoch,
            _ => unreachable!(),
        },
        session_epoch
    );
    let checkpoint = runtime
        .terminal(&TerminalSessionId::new("survives-client"))
        .unwrap()
        .checkpoint()
        .unwrap();
    assert!(viewport_text(&checkpoint.viewport).contains("host-survived"));
    assert_eq!(runtime.placements().len(), 1);

    runtime
        .terminate(&TerminalSessionId::new("survives-client"))
        .unwrap();
}

#[tokio::test]
async fn resize_updates_the_authoritative_checkpoint_and_stale_epochs_are_rejected() {
    let runtime = HostRuntime::new();
    let terminal = runtime
        .spawn(spec("resize", "printf ready; sleep 30"))
        .unwrap();
    let geometry = TerminalGeometry {
        cols: 132,
        rows: 43,
        cell_width: 9,
        cell_height: 18,
    };
    terminal.resize(geometry, 2).unwrap();
    let checkpoint = terminal.checkpoint().unwrap();
    assert_eq!(checkpoint.viewport.geometry, geometry);
    assert_eq!(checkpoint.viewport.geometry_epoch, 2);
    assert!(terminal.resize(geometry, 2).is_err());

    runtime
        .terminate(&TerminalSessionId::new("resize"))
        .unwrap();
}

#[tokio::test]
async fn two_clients_receive_the_same_host_terminal_stream() {
    let runtime = HostRuntime::new();
    let mut first = runtime.subscribe();
    let mut second = runtime.subscribe();
    runtime
        .spawn(spec("fanout", "printf shared-stream; sleep 30"))
        .unwrap();

    let (first_update, second_update) = tokio::join!(
        wait_for_text(&mut first, "shared-stream"),
        wait_for_text(&mut second, "shared-stream"),
    );
    assert_eq!(first_update, second_update);

    runtime
        .terminate(&TerminalSessionId::new("fanout"))
        .unwrap();
}

#[tokio::test]
async fn interactive_lease_is_exclusive_and_released_when_a_client_disconnects() {
    let runtime = HostRuntime::new();
    runtime
        .spawn(spec("lease", "printf lease-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");

    let lease = runtime
        .acquire_lease(
            &TerminalSessionId::new("lease"),
            &first,
            TerminalLeaseMode::Interactive,
        )
        .unwrap();
    assert_eq!(lease.owner, first);
    assert!(
        runtime
            .acquire_lease(
                &TerminalSessionId::new("lease"),
                &second,
                TerminalLeaseMode::Interactive,
            )
            .is_err()
    );
    let observer = runtime
        .acquire_lease(
            &TerminalSessionId::new("lease"),
            &second,
            TerminalLeaseMode::Observer,
        )
        .unwrap();
    assert_eq!(observer.mode, TerminalLeaseMode::Observer);

    runtime.release_client(&first);
    assert!(
        runtime
            .acquire_lease(
                &TerminalSessionId::new("lease"),
                &second,
                TerminalLeaseMode::Interactive,
            )
            .is_ok()
    );
    runtime.terminate(&TerminalSessionId::new("lease")).unwrap();
}

#[tokio::test]
async fn exited_terminal_remains_in_the_catalog_until_the_client_acknowledges_it() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let terminal = runtime.spawn(spec("exit-ack", "exit 7")).unwrap();
    let session_epoch = terminal.session_epoch();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                HostTerminalEvent::Exited {
                    session_id,
                    session_epoch: event_epoch,
                    ..
                } if session_id == TerminalSessionId::new("exit-ack")
                    && event_epoch == session_epoch
            ) {
                break;
            }
        }
    })
    .await
    .expect("terminal exit timeout");

    assert_eq!(runtime.placements().len(), 1);
    assert!(
        runtime
            .acknowledge_terminal_exit(&TerminalSessionId::new("exit-ack"), session_epoch + 1)
            .is_err()
    );
    runtime
        .acknowledge_terminal_exit(&TerminalSessionId::new("exit-ack"), session_epoch)
        .unwrap();
    assert!(runtime.placements().is_empty());
}

#[tokio::test]
async fn shell_mode_runs_the_initial_command_and_keeps_the_shell_alive() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let mut shell_spec = spec("shell-initial-command", "unused");
    shell_spec.execution = TerminalExecutionSpec::Shell {
        program: "/bin/sh".to_string(),
        args: vec!["-i".to_string()],
        initial_command: Some("printf shell-started".to_string()),
    };
    runtime.spawn(shell_spec).unwrap();

    wait_for_text(&mut events, "shell-started").await;
    let terminal = runtime
        .terminal(&TerminalSessionId::new("shell-initial-command"))
        .unwrap();
    assert_eq!(
        terminal.checkpoint().unwrap().viewport.process_state,
        yttt_protocol::terminal::TerminalProcessState::Running
    );
    runtime
        .terminate(&TerminalSessionId::new("shell-initial-command"))
        .unwrap();
}
