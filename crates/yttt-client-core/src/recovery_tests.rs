use super::*;
use yttt_agent_core::{
    AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState, ProviderId,
};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, terminal::*};
use yttt_transport::{
    TransportListener, memory_pair, receive_control, receive_handshake, send_control,
};

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
        channel: ConnectionChannel::Session,
    }
}

/// A session context for Host epoch 1. `shutdown_tx` stops sessions run with it.
struct TestContext {
    session: ClientSessionContext,
    shutdown_tx: watch::Sender<bool>,
}

impl std::ops::Deref for TestContext {
    type Target = ClientSessionContext;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

fn context() -> TestContext {
    let (shutdown_tx, shutdown) = watch::channel(false);
    let (catalog_requests, _) = mpsc::channel(1);
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    TestContext {
        session: ClientSessionContext {
            catalog_requests,
            events,
            terminal_state: Arc::new(RwLock::new(TerminalState {
                host_epoch: Some(1),
                ..TerminalState::default()
            })),
            identity: identity(),
            host_epoch: 1,
            current_host_epoch: Arc::new(RwLock::new(Some(1))),
            catalog: Arc::default(),
            agent_snapshots: Arc::default(),
            project_token_usage: Arc::default(),
            next_request_id: Arc::new(AtomicU64::new(1)),
            diagnostics: Arc::default(),
            visibility: watch::channel(None).1,
            shutdown,
            control: Arc::default(),
            local_control_recovery: Arc::default(),
        },
        shutdown_tx,
    }
}

fn session(stream: TransportStream) -> SessionConnection {
    let (reader, writer) = split(stream);
    let (writer, writer_task) = SessionWriter::spawn(writer);
    SessionConnection {
        reader: SessionReader::new(reader),
        writer,
        writer_task,
        host: AuthenticatedHost {
            host_id: HostId::new("recovery-test"),
            host_epoch: 1,
            selected_version: RESOURCE_PROTOCOL_VERSION,
            connection_sequence: 1,
        },
        buffered: Vec::new(),
    }
}

/// Serves `stream` as a Session connection with idle command queues.
async fn run_session(
    stream: tokio::io::DuplexStream,
    context: &ClientSessionContext,
) -> Option<String> {
    let (_commands, mut commands) = mpsc::channel(1);
    let (_interactive, mut interactive) = mpsc::channel(1);
    let (_catalog, mut catalog) = mpsc::channel(1);
    connected_session(
        session(Box::new(stream)),
        &mut commands,
        &mut interactive,
        &mut catalog,
        context,
    )
    .await
}

fn empty_catalog() -> ResourceCatalog {
    ResourceCatalog {
        profile_id: ProfileId::new("recovery-test"),
        host_id: HostId::new("test"),
        host_epoch: 1,
        revision: 1,
        terminals: Vec::new(),
        ssh_connections: Vec::new(),
        projects: Vec::new(),
    }
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
async fn attach_checkpoint_cannot_replace_a_mirror_owned_by_its_stream() {
    let context = context();
    let newer = viewport(105);
    let id = newer.session_id.clone();
    register_terminal(&context, id.clone());
    context
        .terminal_state
        .write()
        .mirrors
        .insert(id.clone(), TerminalMirror::new(newer.clone()));
    let mut events = context.events.subscribe();
    let mut scrolled = viewport(110);
    scrolled.display_offset = 25;
    seed_terminal_mirror(&scrolled, &context.terminal_state, 1, &context.events);
    assert_eq!(
        context.terminal_state.read().mirrors[&id].viewport(),
        &newer
    );
    assert!(
        events.try_recv().is_err(),
        "only the terminal stream may change an existing mirror"
    );
}

#[test]
fn frames_that_do_not_continue_the_mirror_are_rejected() {
    let context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    handle_terminal_frame(TerminalStreamUpdate::Snapshot(viewport(4)), &context).unwrap();
    for update in [
        TerminalStreamUpdate::ResyncRequired {
            session_id: id.clone(),
            available_from_sequence: 9,
        },
        TerminalStreamUpdate::RawTail {
            session_id: id.clone(),
            session_epoch: 1,
            sequence: 9,
            bytes: Vec::new(),
        },
    ] {
        assert_eq!(handle_terminal_frame(update, &context), Err(()));
    }
    assert_eq!(
        context.terminal_state.read().mirrors[&id]
            .viewport()
            .sequence,
        4
    );
}

#[tokio::test]
async fn a_terminal_stream_gap_disconnects_the_session() {
    let context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    seed_terminal_mirror(&viewport(4), &context.terminal_state, 1, &context.events);
    let (client, mut server) = tokio::io::duplex(8192);
    let host = tokio::spawn(async move {
        send_control(
            &mut server,
            &ControlMessage::Event(HostEvent {
                host_sequence: 1,
                body: ServerEvent::Terminal(TerminalStreamUpdate::ResyncRequired {
                    session_id: viewport(1).session_id,
                    available_from_sequence: 9,
                }),
            }),
        )
        .await
        .unwrap();
        server
    });
    let result = run_session(client, &context).await;
    let _server = host.await.unwrap();
    assert!(result.is_some_and(|message| message.contains("gap")));
}

#[test]
fn catalog_removal_drops_the_terminal_and_its_resubscription() {
    let context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    seed_terminal_mirror(&viewport(4), &context.terminal_state, 1, &context.events);
    let mut events = context.events.subscribe();
    handle_response(
        PendingRequest::Catalog,
        Ok(Response::Resources(empty_catalog())),
        &context,
    );
    let terminal_state = context.terminal_state.read();
    assert!(terminal_state.mirrors.is_empty());
    assert!(
        terminal_state.attached.is_empty(),
        "a removed terminal must not be resubscribed on the next connection"
    );
    assert!(matches!(
        events.try_recv(),
        Ok(ClientEvent::TerminalUnavailable(removed)) if removed == id
    ));
}

#[test]
fn successful_detach_ends_the_resubscription() {
    let context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    let (reply, _response) = oneshot::channel();
    handle_response(
        PendingRequest::User {
            reply,
            detaches: vec![id.clone()],
        },
        Err(ProtocolFailure::new(
            yttt_protocol::FailureCode::InvalidRequest,
            "rejected",
            false,
        )),
        &context,
    );
    assert!(
        context.terminal_state.read().attached.contains(&id),
        "a rejected detach keeps the attachment"
    );
    let (reply, _response) = oneshot::channel();
    handle_response(
        PendingRequest::User {
            reply,
            detaches: vec![id.clone()],
        },
        Ok(Response::TerminalDetached),
        &context,
    );
    let terminal_state = context.terminal_state.read();
    assert!(terminal_state.attached.is_empty());
    assert!(terminal_state.known_sessions.contains(&id));
}

#[tokio::test]
async fn a_new_session_resubscribes_attached_terminals() {
    let context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    let (client, mut server) = tokio::io::duplex(8192);
    let host = tokio::spawn(async move {
        let mut bodies = Vec::new();
        for _ in 0..3 {
            let ControlMessage::Request(request) = receive_control(&mut server).await.unwrap()
            else {
                panic!("expected a request");
            };
            bodies.push(request.body);
        }
        (bodies, server)
    });
    let shutdown = context.shutdown_tx.clone();
    let session = tokio::spawn(async move { run_session(client, &context).await });
    let (bodies, _server) = host.await.unwrap();
    assert!(matches!(bodies[0], Request::ListResources));
    assert!(matches!(bodies[1], Request::ReadAgentSnapshots { .. }));
    assert!(matches!(
        &bodies[2],
        Request::RequestCheckpoint { session_id, .. } if *session_id == id
    ));
    shutdown.send(true).unwrap();
    assert!(session.await.unwrap().is_none());
}

#[test]
fn a_failed_resubscription_is_not_retried() {
    let context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    handle_response(
        PendingRequest::Resubscribe(id.clone()),
        Err(ProtocolFailure::new(
            yttt_protocol::FailureCode::NotFound,
            "missing",
            false,
        )),
        &context,
    );
    assert!(context.terminal_state.read().attached.is_empty());
}

#[test]
fn host_epoch_reset_clears_agents_and_rejects_old_terminal_frames() {
    let context = context();
    let session_id = viewport(9).session_id;
    let mut events = context.events.subscribe();
    register_terminal(&context, session_id.clone());
    seed_terminal_mirror(&viewport(9), &context.terminal_state, 1, &context.events);
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
        &HostEpochState {
            current_host_epoch: &context.current_host_epoch,
            terminal_state: &context.terminal_state,
            catalog: &context.catalog,
            agent_snapshots: &context.agent_snapshots,
            project_token_usage: &context.project_token_usage,
            profile_control: &context.control,
            events: &context.events,
        },
    );
    assert!(context.agent_snapshots.read().is_empty());
    assert!(context.terminal_state.read().mirrors.is_empty());
    assert!(context.terminal_state.read().attached.is_empty());
    assert!(matches!(
        events.try_recv(),
        Ok(ClientEvent::AgentSnapshotsReset { host_epoch: 2 })
    ));

    context
        .terminal_state
        .write()
        .known_sessions
        .insert(session_id.clone());
    seed_terminal_mirror(&viewport(2), &context.terminal_state, 1, &context.events);
    assert!(
        context.terminal_state.read().mirrors.is_empty(),
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

    seed_terminal_mirror(&viewport(1), &context.terminal_state, 2, &context.events);
    assert_eq!(
        context.terminal_state.read().mirrors[&session_id]
            .viewport()
            .sequence,
        1,
        "the restarted Host may reuse a session and session epoch with a lower sequence"
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

#[tokio::test]
async fn cancelling_a_session_closes_its_connection() {
    use tokio::io::AsyncReadExt;
    let context = context();
    let (client, mut server) = tokio::io::duplex(8192);
    let task = tokio::spawn(async move { run_session(client, &context).await });
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
async fn a_silent_session_is_detected_by_its_heartbeat() {
    let context = context();
    let (client, _silent_server) = tokio::io::duplex(8192);
    let result = tokio::time::timeout(Duration::from_secs(45), run_session(client, &context))
        .await
        .expect("silent connection must fail before the test deadline");
    assert!(result.is_some_and(|message| message.contains("heartbeat")));
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
    let mut client = session(Box::new(client));
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
async fn blocked_frame_write_has_a_deadline() {
    let (mut writer, _reader) = tokio::io::duplex(1);
    let result = yttt_transport::send_frame(&mut writer, &[0; 8]).await;
    assert!(
        matches!(result, Err(yttt_transport::WireError::Codec(yttt_protocol::ProtocolCodecError::Io(error))) if error.kind() == std::io::ErrorKind::TimedOut)
    );
}

#[tokio::test(start_paused = true)]
async fn a_responsive_session_stays_connected() {
    let context = context();
    let shutdown = context.shutdown_tx.clone();
    let (client, mut server) = tokio::io::duplex(8192);
    let task = tokio::spawn(async move { run_session(client, &context).await });
    let mut pongs = 0;
    while pongs < 8 {
        let ControlMessage::Request(request) = receive_control(&mut server).await.unwrap() else {
            panic!("expected a request")
        };
        let Request::Ping { sent_millis } = request.body else {
            continue;
        };
        send_control(
            &mut server,
            &ControlMessage::Response(HostResponse {
                request_id: request.request_id,
                result: Ok(Response::Pong {
                    sent_millis,
                    host_millis: 0,
                }),
            }),
        )
        .await
        .unwrap();
        pongs += 1;
    }
    assert!(!task.is_finished());
    shutdown.send(true).unwrap();
    assert!(task.await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn expired_queued_request_is_rejected_before_transmission() {
    let context = context();
    let shutdown = context.shutdown_tx.clone();
    let (client, mut server) = tokio::io::duplex(8192);
    let (commands, mut commands_rx) = mpsc::channel(1);
    let (_interactive, mut interactive_rx) = mpsc::channel(1);
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
        connected_session(
            session(Box::new(client)),
            &mut commands_rx,
            &mut interactive_rx,
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
