use super::*;
use yttt_agent_core::{
    AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState, ProviderId,
};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, terminal::*};
use yttt_transport::{TransportListener, memory_pair, receive_handshake};

fn identity() -> ClientIdentity {
    ClientIdentity {
        expected_environment: None,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: BuildIdentity {
            product_version: "test".into(),
            build_fingerprint: "recovery-test".into(),
            resource_compatibility: "test".into(),
        },
        profile_id: ProfileId::new("recovery-test"),
        client_instance_id: ClientInstanceId::new("recovery-test"),
        host_epoch_hint: Some(1),
        can_force_stop: false,
        channel: ConnectionChannel::Control,
        terminal_session_id: None,
    }
}

fn context(connector: SharedConnector) -> (ClientSessionContext, watch::Sender<bool>) {
    let (shutdown, shutdown_rx) = watch::channel(false);
    let (checkpoint_requests, _) = mpsc::channel(CHECKPOINT_CAPACITY);
    let (catalog_requests, _) = mpsc::channel(1);
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    (
        ClientSessionContext {
            data_connects: Arc::new(Semaphore::new(4)),
            checkpoint_requests,
            catalog_requests,
            events,
            terminal_state: Arc::new(RwLock::new(TerminalState {
                host_epoch: Some(1),
                ..TerminalState::default()
            })),
            connector,
            identity: identity(),
            host_epoch: 1,
            current_host_epoch: Arc::new(RwLock::new(Some(1))),
            token: AuthToken::from_bytes([7; 32]),
            catalog: Arc::default(),
            agent_snapshots: Arc::default(),
            project_token_usage: Arc::default(),
            next_request_id: Arc::new(AtomicU64::new(1)),
            diagnostics: Arc::default(),
            deferred_terminal_events: Arc::default(),
            shutdown: shutdown_rx,
            control: Arc::default(),
            local_control_recovery: Arc::default(),
        },
        shutdown,
    )
}

fn viewport(sequence: u64) -> SemanticViewport {
    SemanticViewport {
        session_id: TerminalSessionId::new("recovery-terminal"),
        session_epoch: 1,
        sequence,
        geometry: TerminalGeometry {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        },
        geometry_epoch: 1,
        scrollback_epoch: 1,
        history_size: 100,
        display_offset: 0,
        rows: Vec::new(),
        cursor: SemanticCursor {
            row: 0,
            column: 0,
            shape: CursorShape::Block,
            visible: true,
            blinking: false,
        },
        modes: TerminalModes {
            bits: 0,
            title: None,
            cwd: None,
        },
        palette: TerminalPalette {
            colors: Vec::new(),
            revision: 0,
        },
        process_state: TerminalProcessState::Running,
        images: Vec::new(),
        placements: Vec::new(),
    }
}

fn agent_snapshot(host_epoch: u64) -> AgentSnapshotUpdate {
    AgentSnapshotUpdate {
        scope: yttt_protocol::agent::AgentHookScope {
            project_id: "project".to_string(),
            tab_id: "tab".to_string(),
            pane_id: "pane".to_string(),
            generation: 1,
        },
        terminal_session_id: TerminalSessionId::new("recovery-terminal"),
        host_epoch,
        sequence: 1,
        snapshot: AgentSnapshot {
            instance_id: AgentInstanceId::new("agent").unwrap(),
            provider_id: ProviderId::from_static("omp"),
            generation: 1,
            process_state: AgentProcessState::Running,
            turn_state: AgentTurnState::Idle,
            waiting_reason: None,
            waiting_message: None,
            task: None,
            current_action: None,
            last_action_failed: false,
            children: Vec::new(),
            session: None,
            process_exit: None,
            state_started_at: 0,
            updated_at: 1,
            board: Default::default(),
        },
    }
}

#[tokio::test]
async fn interrupted_handshake_is_retryable() {
    let (listener, connector) = memory_pair();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        receive_handshake(&mut stream).await.unwrap();
    });
    let result = establish(
        &SharedConnector::new(connector),
        &identity(),
        &AuthToken::from_bytes([7; 32]),
    )
    .await;
    server.await.unwrap();
    assert!(
        matches!(result, Err(ConnectFailure::Retry(_))),
        "temporary handshake I/O failure must remain retryable"
    );
}

#[tokio::test(start_paused = true)]
async fn handshake_timeout_is_retryable() {
    let (listener, connector) = memory_pair();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        receive_handshake(&mut stream).await.unwrap();
        std::future::pending::<()>().await;
    });
    let result = establish(
        &SharedConnector::new(connector),
        &identity(),
        &AuthToken::from_bytes([7; 32]),
    )
    .await;
    server.abort();
    assert!(
        matches!(result, Err(ConnectFailure::Retry(_))),
        "handshake timeout must remain retryable"
    );
}

#[tokio::test]
async fn permanent_transport_rejection_is_fatal() {
    #[derive(Clone)]
    struct PermanentConnector;

    impl TransportConnector for PermanentConnector {
        fn connect(
            &self,
        ) -> yttt_transport::BoxFuture<
            '_,
            Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
        > {
            Box::pin(async {
                Err(yttt_transport::TransportError::Permanent(
                    "invalid credential".to_string(),
                ))
            })
        }
    }

    assert!(matches!(
        establish(
            &SharedConnector::new(PermanentConnector),
            &identity(),
            &AuthToken::from_bytes([7; 32]),
        )
        .await,
        Err(ConnectFailure::Fatal(_))
    ));
}

#[tokio::test]
async fn late_checkpoint_cannot_replace_a_newer_terminal_mirror() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let newer = viewport(105);
    let id = newer.session_id.clone();
    context
        .terminal_state
        .write()
        .mirrors
        .insert(id.clone(), TerminalMirror::new(newer.clone()));
    let mut events = context.events.subscribe();
    handle_response(
        PendingRequest::Checkpoint(id.clone()),
        Ok(Response::TerminalCheckpoint(TerminalCheckpoint {
            viewport: viewport(100),
            raw_replay_tail: Vec::new(),
            raw_tail_start_sequence: 0,
        })),
        &context,
    );
    assert_eq!(
        context.terminal_state.read().mirrors[&id].viewport(),
        &newer
    );
    assert!(
        events.try_recv().is_err(),
        "stale snapshots must not reach the UI"
    );
}

#[tokio::test]
async fn removed_terminal_stops_its_data_worker() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let id = viewport(1).session_id;
    context
        .terminal_state
        .write()
        .known_sessions
        .insert(id.clone());
    let (desired, desired_rx) = watch::channel(true);
    context.terminal_state.write().data_channels.insert(
        id,
        TerminalDataChannel {
            generation: 0,
            desired,
        },
    );
    handle_response(
        PendingRequest::Catalog,
        Ok(Response::Resources(ResourceCatalog {
            profile_id: ProfileId::new("recovery-test"),
            host_id: HostId::new("test"),
            host_epoch: 1,
            revision: 1,
            terminals: Vec::new(),
            ssh_connections: Vec::new(),
            projects: Vec::new(),
        })),
        &context,
    );
    assert!(
        !*desired_rx.borrow(),
        "catalog removal must stop connection retries"
    );
}

#[test]
fn stale_terminal_worker_cannot_restore_a_removed_or_replaced_session() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let session_id = viewport(1).session_id;
    register_terminal(
        &context.terminal_state,
        context.host_epoch,
        session_id.clone(),
    );
    let (old_desired, old_desired_rx) = watch::channel(true);
    context.terminal_state.write().data_channels.insert(
        session_id.clone(),
        TerminalDataChannel {
            generation: 7,
            desired: old_desired,
        },
    );
    apply_terminal_update(
        TerminalStreamUpdate::Snapshot(viewport(8)),
        &context.terminal_state,
        context.host_epoch,
        Some(7),
        &context.events,
    );
    handle_response(
        PendingRequest::Catalog,
        Ok(Response::Resources(ResourceCatalog {
            profile_id: ProfileId::new("recovery-test"),
            host_id: HostId::new("test"),
            host_epoch: 1,
            revision: 1,
            terminals: Vec::new(),
            ssh_connections: Vec::new(),
            projects: Vec::new(),
        })),
        &context,
    );
    assert!(!*old_desired_rx.borrow());
    assert!(context.terminal_state.read().mirrors.is_empty());

    register_terminal(
        &context.terminal_state,
        context.host_epoch,
        session_id.clone(),
    );
    let (replacement_desired, replacement_desired_rx) = watch::channel(true);
    context.terminal_state.write().data_channels.insert(
        session_id.clone(),
        TerminalDataChannel {
            generation: 8,
            desired: replacement_desired,
        },
    );
    assert_eq!(
        apply_terminal_update(
            TerminalStreamUpdate::Snapshot(viewport(9)),
            &context.terminal_state,
            context.host_epoch,
            Some(7),
            &context.events,
        ),
        None
    );
    stop_terminal_data_channel_if_current(
        &context.terminal_state,
        &session_id,
        context.host_epoch,
        7,
    );
    let terminal_state = context.terminal_state.read();
    assert!(terminal_state.mirrors.is_empty());
    assert_eq!(terminal_state.data_channels[&session_id].generation, 8);
    assert!(*replacement_desired_rx.borrow());
}

#[test]
fn host_epoch_reset_clears_agents_and_rejects_old_terminal_frames() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let session_id = viewport(9).session_id;
    let mut events = context.events.subscribe();
    register_terminal(
        &context.terminal_state,
        context.host_epoch,
        session_id.clone(),
    );
    apply_terminal_update(
        TerminalStreamUpdate::Snapshot(viewport(9)),
        &context.terminal_state,
        1,
        None,
        &context.events,
    );
    apply_agent_snapshots(
        vec![agent_snapshot(1)],
        &context.agent_snapshots,
        &context.current_host_epoch,
        1,
        &context.events,
    );
    while events.try_recv().is_ok() {}

    install_host_epoch(
        2,
        &context.current_host_epoch,
        &context.terminal_state,
        &context.catalog,
        &context.agent_snapshots,
        &context.project_token_usage,
        &context.deferred_terminal_events,
        &context.control,
        &context.events,
    );
    assert!(context.agent_snapshots.read().is_empty());
    assert!(context.terminal_state.read().mirrors.is_empty());
    assert!(matches!(
        events.try_recv(),
        Ok(ClientEvent::AgentSnapshotsReset { host_epoch: 2 })
    ));

    register_terminal(&context.terminal_state, 2, session_id.clone());
    assert_eq!(
        apply_terminal_update(
            TerminalStreamUpdate::Snapshot(viewport(2)),
            &context.terminal_state,
            1,
            None,
            &context.events,
        ),
        None,
        "a frame from the old Host cannot recreate a reused terminal session"
    );
    apply_agent_snapshots(
        vec![agent_snapshot(1)],
        &context.agent_snapshots,
        &context.current_host_epoch,
        1,
        &context.events,
    );
    assert!(context.agent_snapshots.read().is_empty());

    apply_terminal_update(
        TerminalStreamUpdate::Snapshot(viewport(1)),
        &context.terminal_state,
        2,
        None,
        &context.events,
    );
    assert_eq!(
        context.terminal_state.read().mirrors[&session_id]
            .viewport()
            .sequence,
        1,
        "the restarted Host may reuse a session and session epoch with a lower sequence"
    );
}

#[tokio::test]
async fn enabling_an_active_data_worker_does_not_cancel_its_frame_read() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let id = viewport(1).session_id;
    register_terminal(&context.terminal_state, context.host_epoch, id.clone());
    let (desired, desired_rx) = watch::channel(true);
    context.terminal_state.write().data_channels.insert(
        id.clone(),
        TerminalDataChannel {
            generation: 0,
            desired,
        },
    );
    start_terminal_data_channel(id, &context);
    assert!(
        !desired_rx.has_changed().unwrap(),
        "an unchanged desired state must not interrupt an in-progress frame"
    );
}

#[tokio::test(start_paused = true)]
async fn stalled_connector_has_a_deadline() {
    let (_listener, connector) = memory_pair();
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        establish(
            &SharedConnector::new(connector),
            &identity(),
            &AuthToken::from_bytes([7; 32]),
        ),
    )
    .await;
    assert!(
        matches!(result, Ok(Err(ConnectFailure::Retry(_)))),
        "a stalled connect must finish with a retryable error"
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_cancels_a_data_worker_stuck_connecting() {
    let (_listener, connector) = memory_pair();
    let (context, shutdown) = context(SharedConnector::new(connector));
    let id = viewport(1).session_id;
    register_terminal(&context.terminal_state, context.host_epoch, id.clone());
    start_terminal_data_channel(id, &context);
    tokio::task::yield_now().await;
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !context.terminal_state.read().data_channels.is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("shutdown must cancel connect immediately");
}

#[tokio::test]
async fn cancelling_interactive_session_releases_its_reader() {
    use tokio::io::AsyncReadExt;
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let (client, mut server) = tokio::io::duplex(1024);
    let (_commands, mut commands_rx) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        connected_interactive_session(Box::new(client), &mut commands_rx, &context).await
    });
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), server.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn silent_interactive_connection_is_detected_without_control_requests() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let (client, _silent_server) = tokio::io::duplex(8192);
    let (_commands, mut commands_rx) = mpsc::channel(1);
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        connected_interactive_session(Box::new(client), &mut commands_rx, &context),
    )
    .await
    .expect("silent connection must fail before the test deadline");
    assert!(result.is_some_and(|message| message.contains("heartbeat")));
}

#[tokio::test]
async fn equal_sequence_viewport_navigation_is_preserved() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let initial = viewport(100);
    let id = initial.session_id.clone();
    context
        .terminal_state
        .write()
        .mirrors
        .insert(id.clone(), TerminalMirror::new(initial.clone()));
    let mut scrolled = initial;
    scrolled.display_offset = 25;
    let (reply, response) = oneshot::channel();
    handle_response(
        PendingRequest::User(reply),
        Ok(Response::TerminalScrolled(TerminalViewportRead {
            viewport: scrolled,
            bottom_line_id: None,
            checkpoint_sequence: 100,
            unseen_output: 0,
        })),
        &context,
    );
    assert!(response.await.unwrap().is_ok());
    assert_eq!(
        context.terminal_state.read().mirrors[&id]
            .viewport()
            .display_offset,
        25
    );
}

fn host_identity() -> yttt_transport::HostIdentity {
    let client = identity();
    yttt_transport::HostIdentity {
        resource_supported: client.supported,
        lifecycle_supported: ProtocolRange::exact(yttt_protocol::LIFECYCLE_PROTOCOL_VERSION),
        build: client.build,
        profile_id: client.profile_id,
        environment_id: "recovery-test".into(),
        credential_generation: 0,
        ingress: yttt_transport::IngressKind::LocalAdmin,
        host_id: HostId::new("recovery-test"),
        host_epoch: 1,
        connection_sequence: 1,
    }
}

#[test]
fn handshake_rejections_have_explicit_retry_policy() {
    use yttt_protocol::RejectReason::*;
    for reason in [StaleHostEpoch, AlreadyConnected, HostShuttingDown] {
        assert!(matches!(
            ConnectFailure::from(HandshakeError::Rejected(reason)),
            ConnectFailure::Retry(_)
        ));
    }
    for reason in [
        VersionMismatch {
            supported: ProtocolRange::exact(1),
        },
        BuildMismatch,
        ProfileMismatch,
        AuthenticationFailed,
        InvalidMessage,
        PermissionDenied,
    ] {
        assert!(matches!(
            ConnectFailure::from(HandshakeError::Rejected(reason)),
            ConnectFailure::Fatal(_)
        ));
    }
    assert!(matches!(
        ConnectFailure::from(HandshakeError::Wire(yttt_transport::WireError::Codec(
            yttt_protocol::ProtocolCodecError::InvalidMagic
        ))),
        ConnectFailure::Fatal(_)
    ));
}

#[test]
fn local_control_recovery_only_targets_an_unowned_profile() {
    use yttt_protocol::session::{ControlContext, ControlStatus, ControlTransfer, TransferPhase};

    let local = ClientInstanceId::new("local-controller");
    let other = ClientInstanceId::new("other-controller");
    let context = ControlContext {
        host_epoch: 1,
        control_epoch: 4,
    };
    let owned = ControlStatus {
        context,
        revision: 1,
        owner: Some(local.clone()),
        transfer: None,
    };
    let recovery = LocalControlRecovery::default();
    recovery.enabled.store(true, Ordering::Release);
    recovery.observe(&owned, &local);
    let unowned = ControlStatus {
        owner: None,
        ..owned.clone()
    };
    assert!(should_reacquire_local_control(&recovery, true, &unowned));

    let already_owned = ControlStatus {
        owner: Some(other.clone()),
        ..unowned.clone()
    };
    assert!(!should_reacquire_local_control(
        &recovery,
        true,
        &already_owned
    ));
    let transfer_pending = ControlStatus {
        transfer: Some(ControlTransfer {
            id: "handoff".to_string(),
            requester: other,
            previous_owner: Some(local.clone()),
            phase: TransferPhase::Preparing,
            deadline_millis: 10,
        }),
        ..owned.clone()
    };
    assert!(!should_reacquire_local_control(
        &recovery,
        true,
        &transfer_pending
    ));

    recovery.observe(&already_owned, &local);
    assert!(
        !recovery.can_recover(),
        "a completed handoff clears recovery"
    );
    recovery.observe(&owned, &local);
    assert!(
        recovery.can_recover(),
        "a later explicit local acquisition restores the opted-in policy"
    );
}

#[tokio::test]
async fn prior_local_controller_reacquires_only_with_the_authenticated_context() {
    use yttt_protocol::session::{ControlContext, ControlStatus, ProfileControlRequest};

    let identity = identity();
    let context = ControlContext {
        host_epoch: 3,
        control_epoch: 8,
    };
    let recovered = ControlStatus {
        context,
        revision: 5,
        owner: Some(identity.client_instance_id.clone()),
        transfer: None,
    };
    let (client, mut server) = tokio::io::duplex(8192);
    let mut client: TransportStream = Box::new(client);
    let server_context = context;
    let server_status = recovered.clone();
    let server = tokio::spawn(async move {
        let ControlMessage::Request(request) = receive_control(&mut server).await.unwrap() else {
            panic!("expected recovery request");
        };
        assert_eq!(request.actor_device_id.as_deref(), Some("recovery-test"));
        assert_eq!(request.control, Some(server_context));
        assert!(matches!(
            request.body,
            Request::ProfileControl(ProfileControlRequest::ReacquireIfUnowned {
                context: request_context,
            }) if request_context == server_context
        ));
        send_control(
            &mut server,
            &ControlMessage::Response(HostResponse {
                request_id: request.request_id,
                result: Ok(Response::ProfileControl(server_status)),
            }),
        )
        .await
        .unwrap();
    });

    assert_eq!(
        reacquire_local_control(&mut client, &identity, context)
            .await
            .unwrap(),
        recovered
    );
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn permanently_rejected_data_worker_stops_retrying() {
    let (listener, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        receive_handshake(&mut stream).await.unwrap();
        yttt_transport::send_handshake(
            &mut stream,
            &yttt_protocol::HandshakeMessage::Rejected(
                yttt_protocol::RejectReason::AuthenticationFailed,
            ),
        )
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .is_err(),
            "permanent errors must not retry"
        );
    });
    register_terminal(
        &context.terminal_state,
        context.host_epoch,
        viewport(1).session_id,
    );
    start_terminal_data_channel(viewport(1).session_id, &context);
    server.await.unwrap();
    assert!(context.terminal_state.read().data_channels.is_empty());
}

#[tokio::test]
async fn fragmented_terminal_frame_survives_repeated_attach() {
    use tokio::io::AsyncWriteExt;
    let (listener, connector) = memory_pair();
    let (context, shutdown) = context(SharedConnector::new(connector));
    let mut events = context.events.subscribe();
    let (partial_tx, partial_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        yttt_transport::server_handshake(
            &mut stream,
            &host_identity(),
            &AuthToken::from_bytes([7; 32]),
        )
        .await
        .unwrap();
        let frame = yttt_protocol::encode_message(
            yttt_protocol::FrameKind::Control,
            &ControlMessage::Event(HostEvent {
                host_sequence: 1,
                body: ServerEvent::Terminal(TerminalStreamUpdate::Snapshot(viewport(7))),
            }),
        )
        .unwrap();
        stream
            .write_all(&frame[..yttt_protocol::HEADER_LEN + 3])
            .await
            .unwrap();
        partial_tx.send(()).unwrap();
        resume_rx.await.unwrap();
        stream
            .write_all(&frame[yttt_protocol::HEADER_LEN + 3..])
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let id = viewport(1).session_id;
    register_terminal(&context.terminal_state, context.host_epoch, id.clone());
    start_terminal_data_channel(id.clone(), &context);
    partial_rx.await.unwrap();
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    start_terminal_data_channel(id.clone(), &context);
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    resume_tx.send(()).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, ClientEvent::TerminalUpdated(_)));
    assert_eq!(
        context.terminal_state.read().mirrors[&id]
            .viewport()
            .sequence,
        7
    );
    shutdown.send(true).unwrap();
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn blocked_frame_write_has_a_deadline() {
    let (mut writer, _reader) = tokio::io::duplex(1);
    let result = yttt_transport::send_frame(&mut writer, &[0; 8]).await;
    assert!(
        matches!(result, Err(yttt_transport::WireError::Codec(yttt_protocol::ProtocolCodecError::Io(error))) if error.kind() == std::io::ErrorKind::TimedOut)
    );
}

#[tokio::test(start_paused = true)]
async fn responsive_legacy_interactive_host_remains_connected() {
    let (_, connector) = memory_pair();
    let (context, shutdown) = context(SharedConnector::new(connector));
    let (client, mut server) = tokio::io::duplex(8192);
    let (_commands, mut commands_rx) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        connected_interactive_session(Box::new(client), &mut commands_rx, &context).await
    });
    for _ in 0..8 {
        let TerminalInteractiveMessage::Request(request) =
            receive_terminal_interactive(&mut server).await.unwrap()
        else {
            panic!("expected heartbeat")
        };
        send_terminal_interactive(
            &mut server,
            &TerminalInteractiveMessage::Response(HostResponse {
                request_id: request.request_id,
                result: Err(ProtocolFailure::new(
                    yttt_protocol::FailureCode::InvalidRequest,
                    "legacy lane",
                    false,
                )),
            }),
        )
        .await
        .unwrap();
    }
    assert!(!task.is_finished());
    shutdown.send(true).unwrap();
    assert!(task.await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn expired_queued_request_is_rejected_before_transmission() {
    let (_, connector) = memory_pair();
    let (context, shutdown) = context(SharedConnector::new(connector));
    let (client, mut server) = tokio::io::duplex(8192);
    let (commands, mut commands_rx) = mpsc::channel(1);
    let (_checkpoints, mut checkpoint_rx) = mpsc::channel(1);
    let (_catalog, mut catalog_rx) = mpsc::channel(1);
    let (reply, response) = oneshot::channel();
    commands
        .send(ClientCommand {
            request_id: Some(100),
            body: Request::Ping { sent_millis: 100 },
            reply: Some(reply),
            control: None,
            deadline: tokio::time::Instant::now(),
        })
        .await
        .unwrap();
    let task = tokio::spawn(async move {
        connected_control_session(
            Box::new(client),
            &mut commands_rx,
            &mut checkpoint_rx,
            &mut catalog_rx,
            &context,
        )
        .await
    });
    for _ in 0..2 {
        receive_control(&mut server).await.unwrap();
    }
    assert!(matches!(
        response.await.unwrap(),
        Err(ClientCoreError::RequestTimeout)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receive_control(&mut server))
            .await
            .is_err()
    );
    shutdown.send(true).unwrap();
    assert!(task.await.unwrap().is_none());
}

#[test]
fn project_token_usage_cache_updates_without_agent_snapshots() {
    let usage = Arc::new(RwLock::new(HashMap::new()));
    let current_host_epoch = Arc::new(RwLock::new(Some(1)));
    let (events, mut received) = broadcast::channel(EVENT_CAPACITY);
    let summary = ProjectTokenUsage {
        project_id: "project".to_string(),
        tokens: Some(0),
        partial: false,
        reported_sources: 1,
        observed_sources: 1,
        updated_at: 4,
    };
    apply_project_token_usage(summary.clone(), &usage, &current_host_epoch, 1, &events);
    assert_eq!(usage.read().get("project"), Some(&summary));
    assert!(matches!(
        received.try_recv().unwrap(),
        ClientEvent::ProjectTokenUsageUpdated(update) if *update == summary
    ));

    let stale = ProjectTokenUsage {
        tokens: Some(99),
        updated_at: 3,
        ..summary
    };
    apply_project_token_usage(stale, &usage, &current_host_epoch, 1, &events);
    assert!(received.try_recv().is_err());
    assert_eq!(usage.read()["project"].tokens, Some(0));
}
#[tokio::test(start_paused = true)]
async fn slow_frame_write_can_exceed_timeout_while_making_progress() {
    use tokio::io::AsyncReadExt;
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let sink = tokio::spawn(async move {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_secs(20)).await;
            assert_eq!(reader.read_u8().await.unwrap(), 7);
        }
    });
    yttt_transport::send_frame(&mut writer, &[7; 4])
        .await
        .unwrap();
    sink.await.unwrap();
}
