#![cfg(unix)]

use std::time::Duration;

use yttt_core::model::ids::{ClientInstanceId, PaneId, ProjectId, TabId, TerminalSessionId};
use yttt_host::{
    runtime::{EXIT_ACK_TTL, HostRuntime},
    terminal::HostTerminalEvent,
};
use yttt_protocol::terminal::{
    SemanticViewport, TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode,
    TerminalProcessState, TerminalSpawnSpec, TerminalStreamUpdate,
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
        query_palette: Vec::new(),
        palette_revision: 1,
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
            let text = match update.update() {
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
                return update.update().clone();
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
    terminal.set_subscriber_count(1);
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
    let terminal = runtime
        .spawn(spec("fanout", "printf shared-stream; sleep 30"))
        .unwrap();
    terminal.set_subscriber_count(2);

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
async fn interactive_lease_transfers_and_notifies_the_previous_owner() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    runtime
        .spawn(spec("lease", "printf lease-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("lease");

    let lease = runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    assert_eq!(lease.owner, first);
    let observer = runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Observer)
        .unwrap();
    assert_eq!(observer.mode, TerminalLeaseMode::Observer);

    let transferred = runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
        .unwrap();
    assert_eq!(transferred.owner, second);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                HostTerminalEvent::LeaseRevoked {
                    session_id: revoked_session,
                    previous_owner,
                } if revoked_session == session_id && previous_owner == first
            ) {
                break;
            }
        }
    })
    .await
    .expect("lease revocation timeout");
    assert!(runtime.validate_lease(&session_id, &first).is_err());
    assert!(runtime.validate_lease(&session_id, &second).is_ok());

    runtime.release_client(&second);
    assert!(
        runtime
            .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
            .is_ok()
    );
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn exited_terminal_remains_in_the_catalog_until_the_client_acknowledges_it() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let terminal = runtime
        .spawn(spec("exit-ack", "printf 'final-before-exit\\n'; exit 7"))
        .unwrap();
    let session_epoch = terminal.session_epoch();

    let final_sequence = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let HostTerminalEvent::Exited {
                session_id,
                session_epoch: event_epoch,
                final_sequence,
                ..
            } = events.recv().await.unwrap()
                && session_id == TerminalSessionId::new("exit-ack")
                && event_epoch == session_epoch
            {
                break final_sequence;
            }
        }
    })
    .await
    .expect("terminal exit timeout");

    let checkpoint = terminal.checkpoint().unwrap();
    assert_eq!(checkpoint.viewport.sequence, final_sequence);
    assert_eq!(
        checkpoint.viewport.process_state,
        TerminalProcessState::Exited { code: Some(7) }
    );
    assert!(viewport_text(&checkpoint.viewport).contains("final-before-exit"));
    assert_eq!(runtime.placements().len(), 1);
    assert!(
        runtime
            .acknowledge_terminal_exit(
                &TerminalSessionId::new("exit-ack"),
                session_epoch + 1,
                final_sequence,
            )
            .is_err()
    );
    assert!(
        runtime
            .acknowledge_terminal_exit(
                &TerminalSessionId::new("exit-ack"),
                session_epoch,
                final_sequence.saturating_sub(1),
            )
            .is_err()
    );
    runtime
        .acknowledge_terminal_exit(
            &TerminalSessionId::new("exit-ack"),
            session_epoch,
            final_sequence,
        )
        .unwrap();
    assert!(runtime.placements().is_empty());
}

#[tokio::test]
async fn unacknowledged_exited_terminal_is_reaped_after_the_client_ttl() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let session_id = TerminalSessionId::new("exit-ttl");
    runtime.spawn(spec(session_id.as_str(), "exit 0")).unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                HostTerminalEvent::Exited {
                    session_id: exited,
                    ..
                } if exited == session_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("terminal exit timeout");
    let exited_at = runtime
        .terminal(&session_id)
        .unwrap()
        .exited_at_millis()
        .unwrap();
    let ttl_millis = EXIT_ACK_TTL.as_millis() as u64;
    assert_eq!(
        runtime.reap_expired_exits(exited_at.saturating_add(ttl_millis).saturating_sub(1)),
        0
    );
    assert!(runtime.terminal(&session_id).is_some());
    assert_eq!(
        runtime.reap_expired_exits(exited_at.saturating_add(ttl_millis)),
        1
    );
    assert!(runtime.terminal(&session_id).is_none());
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
    let terminal = runtime.spawn(shell_spec).unwrap();
    terminal.set_subscriber_count(1);

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

#[tokio::test]
async fn unsubscribed_output_is_parsed_without_semantic_encoding() {
    let runtime = HostRuntime::new();
    let terminal = runtime
        .spawn(spec(
            "unsubscribed",
            "IFS= read -r line; printf 'unsubscribed:%s\\n' \"$line\"; sleep 30",
        ))
        .unwrap();
    terminal.reset_diagnostics();
    terminal.input(b"still-drained\\n".to_vec()).unwrap();

    let diagnostics = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let diagnostics = terminal.diagnostics();
            if diagnostics.bytes_parsed != 0
                && diagnostics.input_to_pty.samples != 0
                && diagnostics.skipped_unsubscribed_captures != 0
            {
                break diagnostics;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("unsubscribed output diagnostics timeout");
    assert_eq!(diagnostics.subscribers, 0);
    assert_eq!(diagnostics.semantic_encode_count, 0);
    assert_eq!(diagnostics.shared_ipc_encode_count, 0);
    assert!(
        diagnostics
            .queues
            .iter()
            .all(|queue| queue.capacity > 0 && queue.current == 0),
        "terminal queue backlog: {:?}",
        diagnostics.queues
    );
    assert!(diagnostics.parser.samples > 0);
    terminal.terminate().unwrap();
}
