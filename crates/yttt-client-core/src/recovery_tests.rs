use super::*;
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
            mirrors: Arc::default(),
            connector,
            identity: identity(),
            token: AuthToken::from_bytes([7; 32]),
            data_channels: Arc::default(),
            known_sessions: Arc::default(),
            catalog: Arc::default(),
            agent_snapshots: Arc::default(),
            next_request_id: Arc::new(AtomicU64::new(1)),
            diagnostics: Arc::default(),
            deferred_terminal_events: Arc::default(),
            shutdown: shutdown_rx,
            control: Arc::default(),
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
async fn late_checkpoint_cannot_replace_a_newer_terminal_mirror() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let newer = viewport(105);
    let id = newer.session_id.clone();
    context
        .mirrors
        .write()
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
    assert_eq!(context.mirrors.read()[&id].viewport(), &newer);
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
    context.known_sessions.write().insert(id.clone());
    let (desired, desired_rx) = watch::channel(true);
    context.data_channels.write().insert(id, desired);
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

#[tokio::test]
async fn enabling_an_active_data_worker_does_not_cancel_its_frame_read() {
    let (_, connector) = memory_pair();
    let (context, _shutdown) = context(SharedConnector::new(connector));
    let id = viewport(1).session_id;
    let (desired, desired_rx) = watch::channel(true);
    context.data_channels.write().insert(id.clone(), desired);
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
    start_terminal_data_channel(viewport(1).session_id, &context);
    tokio::task::yield_now().await;
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !context.data_channels.read().is_empty() {
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
        .mirrors
        .write()
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
    assert_eq!(context.mirrors.read()[&id].viewport().display_offset, 25);
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
    start_terminal_data_channel(viewport(1).session_id, &context);
    server.await.unwrap();
    assert!(context.data_channels.read().is_empty());
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
    assert_eq!(context.mirrors.read()[&id].viewport().sequence, 7);
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
