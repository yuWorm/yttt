#![cfg(unix)]

use std::time::Duration;

use yttt_core::model::ids::{ClientInstanceId, ProjectId, TerminalSessionId};
use yttt_host::{
    runtime::{
        EXIT_ACK_TTL, HostRuntime, HostRuntimeConfig, HostRuntimeError, TerminalControlOutcome,
        UnattendedTakeoverPolicy,
    },
    terminal::{HostTerminalEvent, ReplayBudget},
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

#[tokio::test]
async fn second_interactive_acquire_returns_conflict_instead_of_preempting() {
    let runtime = HostRuntime::new();
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

    let conflict = runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
        .unwrap_err();
    assert!(matches!(
        conflict,
        HostRuntimeError::LeaseConflict { holder, .. } if holder == first
    ));
    assert!(runtime.validate_lease(&session_id, &first).is_ok());
    assert!(runtime.validate_lease(&session_id, &second).is_err());
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn idle_ttl_does_not_transfer_control_without_negotiation() {
    let runtime = HostRuntime::new();
    runtime.set_now_millis(1_000);
    runtime
        .spawn(spec("idle-lease", "printf idle-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("idle-lease");

    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    runtime.set_now_millis(1_000 + 20_000);
    let conflict = runtime
        .acquire_lease(&session_id, &second, TerminalLeaseMode::Interactive)
        .unwrap_err();
    assert!(matches!(conflict, HostRuntimeError::LeaseConflict { .. }));
    assert!(runtime.validate_lease(&session_id, &first).is_ok());
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn request_terminal_control_notifies_holder_and_transfers_on_release() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    runtime
        .spawn(spec("negotiate", "printf negotiate-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("negotiate");

    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    let pending = runtime
        .request_terminal_control(&session_id, &second)
        .unwrap();
    assert_eq!(
        pending,
        TerminalControlOutcome::Pending {
            holder: first.clone()
        }
    );
    wait_for_event(&mut events, |event| {
        matches!(
            event,
            HostTerminalEvent::ControlRequested {
                session_id: requested,
                holder,
                requester,
            } if *requested == session_id && *holder == first && *requester == second
        )
    })
    .await;

    runtime.release_control(&session_id, &first).unwrap();
    wait_for_event(&mut events, |event| {
        matches!(
            event,
            HostTerminalEvent::LeaseReleased {
                session_id: released,
                previous_owner,
            } if *released == session_id && *previous_owner == first
        )
    })
    .await;
    wait_for_event(&mut events, |event| {
        matches!(
            event,
            HostTerminalEvent::ControlGranted { session_id: granted, lease }
                if *granted == session_id && lease.owner == second
        )
    })
    .await;
    assert!(runtime.validate_lease(&session_id, &first).is_err());
    assert!(runtime.validate_lease(&session_id, &second).is_ok());
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn after_idle_policy_grants_pending_request_and_emits_expired() {
    let runtime = HostRuntime::new_with_config(HostRuntimeConfig {
        takeover_policy: UnattendedTakeoverPolicy::AfterIdle {
            idle: Duration::from_secs(5),
        },
        control_request_timeout: Duration::from_secs(30),
        replay_budget: ReplayBudget::default(),
    });
    let mut events = runtime.subscribe();
    runtime.set_now_millis(1_000);
    runtime
        .spawn(spec("idle-takeover", "printf idle-takeover; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("idle-takeover");

    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    runtime.set_now_millis(1_000 + 6_000);
    let granted = runtime
        .request_terminal_control(&session_id, &second)
        .unwrap();
    let TerminalControlOutcome::Granted(lease) = granted else {
        panic!("expected idle policy to grant control: {granted:?}");
    };
    assert_eq!(lease.owner, second);
    wait_for_event(&mut events, |event| {
        matches!(
            event,
            HostTerminalEvent::LeaseExpired {
                session_id: expired,
                previous_owner,
            } if *expired == session_id && *previous_owner == first
        )
    })
    .await;
    assert!(runtime.validate_lease(&session_id, &first).is_err());
    assert!(runtime.validate_lease(&session_id, &second).is_ok());
    runtime.terminate(&session_id).unwrap();
}

#[tokio::test]
async fn disconnected_holder_transfers_to_pending_requester() {
    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    runtime
        .spawn(spec("disconnect", "printf disconnect-ready; sleep 30"))
        .unwrap();
    let first = ClientInstanceId::new("client-1");
    let second = ClientInstanceId::new("client-2");
    let session_id = TerminalSessionId::new("disconnect");

    runtime
        .acquire_lease(&session_id, &first, TerminalLeaseMode::Interactive)
        .unwrap();
    runtime
        .request_terminal_control(&session_id, &second)
        .unwrap();
    runtime.release_client(&first);
    wait_for_event(&mut events, |event| {
        matches!(
            event,
            HostTerminalEvent::ControlGranted { session_id: granted, lease }
                if *granted == session_id && lease.owner == second
        )
    })
    .await;
    assert!(runtime.validate_lease(&session_id, &second).is_ok());
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

#[tokio::test]
async fn control_checkpoint_stays_within_the_frame_limit_after_raw_replay_wraps() {
    use yttt_host::terminal::RAW_REPLAY_BYTES;
    use yttt_protocol::{
        ControlMessage, FrameKind, HostResponse, MAX_FRAME_BYTES, Response, TerminalLease,
        encode_message,
    };

    let runtime = HostRuntime::new();
    let mut events = runtime.subscribe();
    let terminal = runtime
        .spawn(spec("replay-wrap", "printf visible-marker; sleep 30"))
        .unwrap();
    terminal.set_subscriber_count(1);
    wait_for_text(&mut events, "visible-marker").await;
    terminal.append_raw_replay_for_tests(&vec![b'x'; RAW_REPLAY_BYTES.saturating_mul(2)]);

    let checkpoint = terminal.control_checkpoint().unwrap();
    assert!(checkpoint.raw_replay_tail.is_empty());
    assert!(viewport_text(&checkpoint.viewport).contains("visible-marker"));
    assert!(checkpoint.raw_tail_start_sequence >= RAW_REPLAY_BYTES as u64);
    let encoded = encode_message(
        FrameKind::Control,
        &ControlMessage::Response(HostResponse {
            request_id: 1,
            result: Ok(Response::TerminalAttached {
                lease: TerminalLease {
                    session_id: TerminalSessionId::new("replay-wrap"),
                    owner: ClientInstanceId::new("client"),
                    mode: TerminalLeaseMode::Interactive,
                    lease_epoch: 1,
                },
                checkpoint,
            }),
        }),
    )
    .expect("wrapped-replay attach response must encode");
    assert!(encoded.len() <= MAX_FRAME_BYTES);
    assert!(
        terminal.diagnostics().checkpoint_encode_high_water > 0,
        "control checkpoint encode diagnostics must be recorded"
    );
    assert!(matches!(
        terminal.raw_replay_chunks(0),
        Err(yttt_host::terminal::HostedTerminalError::ResyncRequired {
            requested: 0,
            available_from_sequence,
        }) if available_from_sequence >= RAW_REPLAY_BYTES as u64
    ));

    runtime
        .terminate(&TerminalSessionId::new("replay-wrap"))
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
