#![cfg(unix)]

use std::time::Duration;

use yttt_core::model::ids::{ClientInstanceId, ProjectId, TerminalSessionId};
use yttt_host::{
    runtime::{EXIT_ACK_TTL, HostRuntime, HostRuntimeError},
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
        cwd: yttt_protocol::ProjectRelativePath::root(),
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

async fn wait_for_event(
    events: &mut tokio::sync::broadcast::Receiver<HostTerminalEvent>,
    mut predicate: impl FnMut(&HostTerminalEvent) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if predicate(&event) {
                return;
            }
        }
    })
    .await
    .expect("runtime event timeout");
}

fn controller_changed(expected: &ClientInstanceId) -> impl FnMut(&HostTerminalEvent) -> bool + '_ {
    move |event| {
        matches!(
            event,
            HostTerminalEvent::ControllerChanged(state)
                if state.controller.as_ref() == Some(expected)
        )
    }
}

#[tokio::test]
async fn the_first_client_to_write_becomes_the_terminal_controller() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe_control();
    runtime
        .spawn(spec("lease", "printf lease-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let session_id = TerminalSessionId::new("lease");
    assert_eq!(runtime.terminal_controller().controller, None);

    let observer = runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Observer)
        .unwrap();
    assert_eq!(observer.mode, TerminalLeaseMode::Observer);
    assert_eq!(runtime.terminal_controller().controller, None);

    let lease = runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    assert_eq!(lease.owner, first);
    assert_eq!(
        runtime.terminal_controller().controller,
        Some(first.clone())
    );
    wait_for_event(&mut events, controller_changed(&first)).await;
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn a_new_client_takes_control_of_every_terminal() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe_control();
    runtime
        .spawn(spec("takeover-a", "printf a-ready; sleep 30"))
        .unwrap();
    runtime
        .spawn(spec("takeover-b", "printf b-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let a = TerminalSessionId::new("takeover-a");
    let b = TerminalSessionId::new("takeover-b");
    runtime
        .acquire_lease(&a, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    runtime
        .acquire_lease(&b, &first, TerminalLeaseMode::Interactive)
        .unwrap();

    let lease = runtime
        .acquire_lease(&a, &second, TerminalLeaseMode::Interactive)
        .unwrap();
    assert_eq!(lease.owner, second);
    assert_eq!(
        runtime.terminal_controller().controller,
        Some(second.clone())
    );
    let mut sent = Vec::new();
    while let Ok(event) = events.try_recv() {
        sent.push(event);
    }
    assert!(sent.iter().any(controller_changed(&second)));
    for session_id in [&a, &b] {
        assert!(runtime.validate_lease(session_id, &first).is_err());
        assert!(runtime.validate_lease(session_id, &second).is_ok());
        assert!(sent.iter().any(|event| matches!(
            event,
            HostTerminalEvent::LeaseRevoked { session_id: revoked, previous_owner }
                if revoked == session_id && *previous_owner == first
        )));
    }
    runtime.terminate(&a).unwrap();
    runtime.terminate(&b).unwrap();
}

#[tokio::test]
async fn a_client_that_lost_control_observes_until_it_takes_control_back() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe_control();
    runtime
        .spawn(spec("take-back", "printf take-back; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("take-back");
    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
        .unwrap();

    // Asking again, as a reconnecting pane does, does not take control back.
    let conflict = runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap_err();
    assert!(matches!(
        conflict,
        HostRuntimeError::LeaseConflict { holder, .. } if holder == second
    ));
    assert!(
        runtime
            .request_terminal_control(&session_id, &first)
            .is_err()
    );
    assert!(matches!(
        runtime.claim_terminal_control(&first),
        Err(HostRuntimeError::NotTerminalController { controller }) if controller == second
    ));

    let taken = runtime.take_terminal_control(&first);
    assert_eq!(taken.controller, Some(first.clone()));
    assert!(runtime.validate_lease(&session_id, &first).is_ok());
    assert!(runtime.validate_lease(&session_id, &second).is_err());
    wait_for_event(&mut events, |event| {
        matches!(
            event,
            HostTerminalEvent::ControlGranted { session_id: granted, lease }
                if *granted == session_id && lease.owner == first
        )
    })
    .await;
    assert!(
        runtime
            .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
            .is_err()
    );
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn a_suspended_controller_keeps_control_until_it_resumes() {
    let runtime = HostRuntime::new();
    runtime
        .spawn(spec("suspend", "printf suspend-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("suspend");
    runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
        .unwrap();
    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    let lease = runtime.validate_lease(&session_id, &first).unwrap();

    runtime.suspend_client(&first);
    assert_eq!(runtime.lease(&session_id, &first), Some(lease.clone()));
    assert!(
        runtime
            .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
            .is_err()
    );
    runtime.resume_client(&first);
    assert_eq!(
        runtime
            .validate_lease(&session_id, &first)
            .unwrap()
            .lease_epoch,
        lease.lease_epoch
    );
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn a_departed_controller_hands_control_to_the_latest_present_viewer() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe_control();
    runtime
        .spawn(spec("departed", "printf departed; sleep 30"))
        .unwrap();
    let controller = ClientInstanceId::new("controller");
    let older = ClientInstanceId::new("older-viewer");
    let newer = ClientInstanceId::new("newer-viewer");
    let probe = ClientInstanceId::new("probe");
    let session_id = TerminalSessionId::new("departed");
    for client in [&controller, &older, &newer, &probe] {
        runtime.resume_client(client);
    }
    runtime
        .acquire_lease(&session_id, &controller, TerminalLeaseMode::Interactive)
        .unwrap();
    // A Client that renders no terminal (a probe) is not offered control.
    runtime.register_attachment(&session_id, &older);
    runtime.register_attachment(&session_id, &newer);

    runtime.suspend_client(&controller);
    runtime.release_client(&controller);
    assert_eq!(
        runtime.terminal_controller().controller,
        Some(newer.clone())
    );
    assert!(runtime.validate_lease(&session_id, &newer).is_ok());
    wait_for_event(&mut events, controller_changed(&newer)).await;

    runtime.release_client(&newer);
    assert_eq!(
        runtime.terminal_controller().controller,
        Some(older.clone())
    );
    runtime.release_client(&older);
    assert_eq!(runtime.terminal_controller().controller, None);
    // With nobody in control, the next Client to write takes it.
    runtime
        .acquire_lease(&session_id, &probe, TerminalLeaseMode::Interactive)
        .unwrap();
    assert_eq!(runtime.terminal_controller().controller, Some(probe));
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn lease_events_are_not_pushed_out_by_a_flood_of_frames() {
    let runtime = HostRuntime::new();
    // Never read, so frame events overflow it.
    let _frames = runtime.subscribe();
    let mut control = runtime.subscribe_control();
    runtime
        .spawn(spec(
            "flood",
            "i=0; while [ $i -lt 3000 ]; do echo flood-$i; i=$((i+1)); done; sleep 30",
        ))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("flood");
    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
        .unwrap();
    wait_for_event(&mut control, |event| {
        matches!(
            event,
            HostTerminalEvent::LeaseRevoked { session_id: revoked, previous_owner }
                if *revoked == session_id && *previous_owner == first
        )
    })
    .await;
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
async fn a_command_that_returns_to_its_shell_leaves_the_shell_usable() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let session = TerminalSessionId::new("resumed-in-shell");
    let mut shell_spec = spec("resumed-in-shell", "unused");
    shell_spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "printf".to_string(),
        args: vec![
            "%s-%s\\n".to_string(),
            "resumed".to_string(),
            "agent".to_string(),
        ],
        return_to_shell: true,
    };
    let terminal = runtime.spawn(shell_spec).unwrap();
    terminal.set_subscriber_count(1);
    wait_for_text(&mut events, "resumed-agent").await;

    terminal
        .input(b"printf 'shell-%s\\n' alive\r".to_vec())
        .unwrap();
    wait_for_text(&mut events, "shell-alive").await;
    assert_eq!(
        terminal.checkpoint().unwrap().viewport.process_state,
        yttt_protocol::terminal::TerminalProcessState::Running
    );
    runtime.terminate(&session).unwrap();
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

#[tokio::test]
async fn current_viewport_catches_up_on_output_nobody_rendered() {
    let runtime = HostRuntime::new();
    let terminal = runtime
        .spawn(spec("hidden-output", "printf hidden-marker; sleep 30"))
        .unwrap();
    terminal.set_subscriber_count(0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let diagnostics = terminal.diagnostics();
        if diagnostics.skipped_unsubscribed_captures != 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "output capture was never skipped: {diagnostics:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !terminal
            .latest_viewport()
            .is_some_and(|viewport| viewport_text(&viewport).contains("hidden-marker")),
        "the published viewport must still lag the skipped output"
    );

    let viewport = terminal.current_viewport(0).unwrap();
    assert!(viewport_text(&viewport).contains("hidden-marker"));

    runtime
        .terminate(&TerminalSessionId::new("hidden-output"))
        .unwrap();
}

#[tokio::test]
async fn attachment_lag_while_scrolled_is_exported_in_diagnostics() {
    let runtime = HostRuntime::new();
    runtime
        .spawn(spec("lag-diag", "printf lag-ready; sleep 30"))
        .unwrap();
    let client = ClientInstanceId::new("observer");
    let session_id = TerminalSessionId::new("lag-diag");
    runtime.register_attachment(&session_id, &client);
    let _attached = runtime.attachment_receiver(&session_id, &client).unwrap();
    runtime.update_attachment_display_offset(&session_id, &client, 6);
    runtime.note_attachment_lag(&session_id, &client, 17, true);
    let snapshot = runtime.attachment_resync_diagnostics();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].session_id, "lag-diag");
    assert_eq!(snapshot[0].client_id, "observer");
    assert_eq!(snapshot[0].lagged_events, 17);
    assert!(snapshot[0].pending_scroll_resync);
    assert_eq!(
        snapshot[0].last_resync_reason.as_deref(),
        Some("scrolled_lagged")
    );
    runtime.mark_attachment_resync(
        &session_id,
        &client,
        yttt_host::runtime::AttachmentResyncReason::ViewportRead,
    );
    let snapshot = runtime.attachment_resync_diagnostics();
    assert!(!snapshot[0].pending_scroll_resync);
    assert_eq!(
        snapshot[0].last_resync_reason.as_deref(),
        Some("viewport_read")
    );
    runtime.terminate(&session_id).unwrap();
}
