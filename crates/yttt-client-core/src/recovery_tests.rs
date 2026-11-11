use super::*;
use yttt_agent_core::{
    AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState, ProviderId,
};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{
    BuildIdentity, HostResponse, ProtocolRange, RESOURCE_PROTOCOL_VERSION, terminal::*,
};
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
            workspace_layouts: Arc::default(),
            next_request_id: Arc::new(AtomicU64::new(1)),
            diagnostics: Arc::default(),
            visibility: watch::channel(None).1,
            board_interest: watch::channel(false).1,
            probe: Arc::default(),
            shutdown,
            link: LinkProfile::LOCAL,
            agent_boards: Arc::default(),
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
            turn_seq: 0,
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
        board_omitted: false,
    }
}

fn board_message(id: usize) -> yttt_agent_core::AgentBoardMessage {
    yttt_agent_core::AgentBoardMessage {
        id: format!("message-{id}"),
        execution_id: "run".into(),
        role: yttt_agent_core::AgentMessageRole::Assistant,
        text: format!("text {id}"),
        timestamp: id as u64,
        truncated: false,
    }
}

fn agent_state(sequence: u64, messages: usize) -> AgentSnapshotUpdate {
    let mut update = agent_snapshot(1);
    update.sequence = sequence;
    update.snapshot.board.messages = (0..messages).map(board_message).collect();
    update
}

fn agent_event(body: ServerEvent) -> HostEvent {
    HostEvent {
        host_sequence: 1,
        body,
    }
}

fn agent_patch(
    base: &AgentSnapshotUpdate,
    base_sequence: u64,
    next: &AgentSnapshotUpdate,
) -> HostEvent {
    let mut update = next.clone();
    update.snapshot.board = Default::default();
    agent_event(ServerEvent::AgentSnapshotPatch(Box::new(
        yttt_protocol::agent::AgentSnapshotPatch {
            update,
            base_sequence,
            board: next.snapshot.board.patch_from(&base.snapshot.board),
        },
    )))
}

fn delivered_board(event: HostEvent) -> (u64, Vec<yttt_agent_core::AgentBoardMessage>) {
    match event.body {
        ServerEvent::AgentSnapshot(update) => (update.sequence, update.snapshot.board.messages),
        other => panic!("expected a whole Agent snapshot, got {other:?}"),
    }
}

#[test]
fn an_agent_board_patch_is_delivered_as_the_whole_snapshot() {
    let context = context();
    let first = agent_state(1, 3);
    let second = agent_state(2, 4);
    let third = agent_state(3, 6);
    resolve_agent_board(
        agent_event(ServerEvent::AgentSnapshot(Box::new(first.clone()))),
        &context,
    )
    .unwrap();
    let delivered = resolve_agent_board(agent_patch(&first, 1, &second), &context).unwrap();
    assert_eq!(
        delivered_board(delivered),
        (2, second.snapshot.board.messages.clone())
    );
    let delivered = resolve_agent_board(agent_patch(&second, 2, &third), &context).unwrap();
    assert_eq!(
        delivered_board(delivered),
        (3, third.snapshot.board.messages)
    );
}

#[test]
fn an_agent_board_patch_for_another_board_ends_the_connection() {
    let context = context();
    let first = agent_state(1, 3);
    let second = agent_state(2, 4);
    assert!(resolve_agent_board(agent_patch(&first, 1, &second), &context).is_err());
    resolve_agent_board(
        agent_event(ServerEvent::AgentSnapshot(Box::new(first.clone()))),
        &context,
    )
    .unwrap();
    assert!(resolve_agent_board(agent_patch(&first, 7, &second), &context).is_err());
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
        &Credential::from(yttt_transport::AuthToken::from_bytes([7; 32])),
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
        &Credential::from(yttt_transport::AuthToken::from_bytes([7; 32])),
    )
    .await;
    server.abort();
    assert!(
        matches!(result, Err(ConnectFailure::Retry(_))),
        "handshake timeout must remain retryable"
    );
}

/// Hands out its inner connector's streams and records each reported handshake.
struct SettleRecorder {
    inner: yttt_transport::MemoryConnector,
    settled: Arc<parking_lot::Mutex<Vec<bool>>>,
}

impl TransportConnector for SettleRecorder {
    fn connect(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        self.inner.connect()
    }

    fn handshake_settled(&self, answered: bool) {
        self.settled.lock().push(answered);
    }
}

#[tokio::test(start_paused = true)]
async fn a_handshake_the_host_never_answers_is_reported_to_the_connector() {
    let (listener, inner) = memory_pair();
    let settled = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        receive_handshake(&mut stream).await.unwrap();
        std::future::pending::<()>().await;
    });
    let connector = SettleRecorder {
        inner,
        settled: settled.clone(),
    };
    let result = establish(
        &SharedConnector::new(connector),
        &identity(),
        &Credential::from(yttt_transport::AuthToken::from_bytes([7; 32])),
    )
    .await;
    server.abort();
    assert!(matches!(result, Err(ConnectFailure::Retry(_))));
    assert_eq!(*settled.lock(), vec![false]);
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
            &Credential::from(yttt_transport::AuthToken::from_bytes([7; 32])),
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
            opens: None,
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
            opens: None,
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

#[tokio::test]
async fn a_session_asks_for_agent_boards_before_reading_snapshots_and_when_asked() {
    let mut context = context();
    let (interest, board_interest) = watch::channel(true);
    context.session.board_interest = board_interest;
    // One Agent held whole, one held without its board.
    let mut summary = agent_state(4, 0);
    summary.scope.pane_id = "summary".into();
    summary.board_omitted = true;
    apply_agent_snapshots(
        vec![agent_state(3, 1), summary],
        &context.agent_snapshots,
        &context.current_host_epoch,
        1,
        &context.events,
    );
    let (client, mut server) = tokio::io::duplex(8192);
    let host = tokio::spawn(async move {
        let mut bodies = Vec::new();
        let mut withdrawn = false;
        loop {
            let ControlMessage::Request(request) = receive_control(&mut server).await.unwrap()
            else {
                panic!("expected a request");
            };
            let watch = match request.body {
                Request::SetAgentBoardInterest { watch } => Some(watch),
                _ => None,
            };
            bodies.push(request.body);
            withdrawn |= watch == Some(false);
            if withdrawn && watch == Some(true) {
                let ControlMessage::Request(request) = receive_control(&mut server).await.unwrap()
                else {
                    panic!("expected a request");
                };
                bodies.push(request.body);
                break;
            }
        }
        (bodies, server)
    });
    let shutdown = context.shutdown_tx.clone();
    let session = tokio::spawn(async move { run_session(client, &context).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    interest.send_replace(false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    interest.send_replace(true);
    let (bodies, _server) = host.await.unwrap();
    let asked = bodies
        .iter()
        .position(|body| matches!(body, Request::SetAgentBoardInterest { watch: true }))
        .expect("a watching session asks for boards");
    let read = bodies
        .iter()
        .position(|body| matches!(body, Request::ReadAgentSnapshots { .. }))
        .expect("the session reads Agent snapshots");
    assert!(asked < read, "{bodies:?}");
    // Asking again re-reads only the Agent held without its board.
    let [
        Request::SetAgentBoardInterest { watch: false },
        Request::SetAgentBoardInterest { watch: true },
        Request::ReadAgentSnapshots { acknowledged },
    ] = &bodies[bodies.len() - 3..]
    else {
        panic!("{bodies:?}");
    };
    assert_eq!(acknowledged.len(), 1);
    assert_eq!(acknowledged[0].sequence, 3);
    shutdown.send(true).unwrap();
    assert!(session.await.unwrap().is_none());
}

/// A Host that reads requests and answers Pings only when `answers` is set.
async fn serve_pings(mut server: tokio::io::DuplexStream, answers: bool) {
    while let Ok(message) = receive_control(&mut server).await {
        if let ControlMessage::Request(ClientRequest {
            request_id,
            body: Request::Ping { sent_millis },
            ..
        }) = message
            && answers
        {
            let pong = ControlMessage::Response(HostResponse {
                request_id,
                result: Ok(Response::Pong {
                    sent_millis,
                    host_millis: 0,
                }),
            });
            if send_control(&mut server, &pong).await.is_err() {
                return;
            }
        }
    }
}

/// A path that died while the system slept: the Host never hears the probe.
#[tokio::test(start_paused = true)]
async fn a_probed_session_that_hears_nothing_is_replaced_within_the_probe_timeout() {
    let mut context = context();
    context.session.link = LinkProfile::REMOTE;
    let probe = context.probe.clone();
    let (client, server) = tokio::io::duplex(64 * 1024);
    let _host = tokio::spawn(serve_pings(server, false));
    let started = tokio::time::Instant::now();
    let session = tokio::spawn(async move { run_session(client, &context).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    probe.notify_one();
    let reason = session.await.unwrap().expect("the session ends");
    assert!(reason.contains("did not answer a link check"), "{reason}");
    // Without the probe the link would wait out its 20 s liveness timeout.
    assert!(
        started.elapsed() <= Duration::from_secs(1) + PROBE_TIMEOUT + Duration::from_millis(100)
    );
}

#[tokio::test(start_paused = true)]
async fn a_probed_session_that_answers_stays_up() {
    let mut context = context();
    context.session.link = LinkProfile::REMOTE;
    let probe = context.probe.clone();
    let shutdown = context.shutdown_tx.clone();
    let (client, server) = tokio::io::duplex(64 * 1024);
    let _host = tokio::spawn(serve_pings(server, true));
    let session = tokio::spawn(async move { run_session(client, &context).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    probe.notify_one();
    tokio::time::sleep(PROBE_TIMEOUT * 3).await;
    assert!(!session.is_finished(), "{:?}", session.await);
    shutdown.send(true).unwrap();
    assert!(session.await.unwrap().is_none());
}

#[test]
fn a_snapshot_without_its_board_keeps_the_board_held() {
    let context = context();
    let apply = |update: AgentSnapshotUpdate| {
        apply_agent_snapshots(
            vec![update],
            &context.agent_snapshots,
            &context.current_host_epoch,
            1,
            &context.events,
        );
    };
    let held = |context: &TestContext| {
        context
            .agent_snapshots
            .read()
            .values()
            .next()
            .cloned()
            .unwrap()
    };
    apply(agent_state(1, 3));
    let mut omitted = agent_state(2, 0);
    omitted.board_omitted = true;
    omitted.snapshot.updated_at = 2;
    apply(omitted.clone());
    let kept = held(&context);
    assert_eq!((kept.sequence, kept.snapshot.updated_at), (2, 2));
    assert_eq!(kept.snapshot.board.messages.len(), 3);
    assert!(kept.board_omitted);
    // The same state with its board replaces it; the omitted one again does not.
    apply(agent_state(2, 5));
    assert_eq!(held(&context).snapshot.board.messages.len(), 5);
    apply(omitted);
    assert_eq!(held(&context).snapshot.board.messages.len(), 5);
    assert!(!held(&context).board_omitted);
}

#[test]
fn a_snapshot_without_its_board_drops_the_patch_base() {
    let context = context();
    resolve_agent_board(
        agent_event(ServerEvent::AgentSnapshot(Box::new(agent_state(1, 2)))),
        &context,
    )
    .unwrap();
    let mut omitted = agent_state(2, 0);
    omitted.board_omitted = true;
    resolve_agent_board(
        agent_event(ServerEvent::AgentSnapshot(Box::new(omitted))),
        &context,
    )
    .unwrap();
    // The empty board of the omitted snapshot is no base for a patch.
    let next = agent_state(3, 3);
    let patch =
        ServerEvent::AgentSnapshotPatch(Box::new(yttt_protocol::agent::AgentSnapshotPatch {
            update: next.clone(),
            base_sequence: 2,
            board: next
                .snapshot
                .board
                .patch_from(&agent_state(2, 0).snapshot.board),
        }));
    assert!(resolve_agent_board(agent_event(patch), &context).is_err());
}

#[tokio::test]
async fn a_new_session_reports_visibility_before_it_resubscribes_without_a_checkpoint() {
    let mut context = context();
    let id = viewport(1).session_id;
    register_terminal(&context, id.clone());
    let (_visibility, visibility) = watch::channel(Some(Arc::new(VisibleTerminals {
        revision: 3,
        sessions: Vec::new(),
    })));
    context.session.visibility = visibility;
    let (client, mut server) = tokio::io::duplex(8192);
    let host = tokio::spawn(async move {
        let mut bodies = Vec::new();
        for _ in 0..4 {
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
    let visibility = bodies
        .iter()
        .position(|body| matches!(body, Request::SetTerminalVisibility { revision: 3, .. }))
        .expect("visibility is reported");
    let resubscribe = bodies
        .iter()
        .position(|body| {
            matches!(
                body,
                Request::RequestCheckpoint { session_id, subscribe_only: true, .. }
                    if *session_id == id
            )
        })
        .expect("the terminal is resubscribed without a checkpoint");
    assert!(visibility < resubscribe);
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
            workspace_layouts: &context.workspace_layouts,
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
            &Credential::from(yttt_transport::AuthToken::from_bytes([7; 32])),
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

#[tokio::test(start_paused = true)]
async fn inbound_traffic_keeps_a_session_alive_while_its_pong_is_late() {
    let context = context();
    let shutdown = context.shutdown_tx.clone();
    let (client, server) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(async move { run_session(client, &context).await });
    let (mut requests, mut host) = tokio::io::split(server);
    let _drain = tokio::spawn(async move { while receive_control(&mut requests).await.is_ok() {} });
    // The Host streams for well past the liveness timeout but never answers Ping.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        send_control(
            &mut host,
            &ControlMessage::Event(HostEvent {
                host_sequence: 1,
                body: ServerEvent::TerminalLeaseRevoked {
                    session_id: TerminalSessionId::new("busy"),
                    previous_owner: ClientInstanceId::new("other"),
                },
            }),
        )
        .await
        .unwrap();
    }
    assert!(!task.is_finished(), "a streaming Host is alive");
    shutdown.send(true).unwrap();
    assert!(task.await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_streaming_host_that_never_answers_is_dropped_eventually() {
    let context = context();
    let (client, server) = tokio::io::duplex(1 << 16);
    let task = tokio::spawn(async move { run_session(client, &context).await });
    let (mut requests, mut host) = tokio::io::split(server);
    let _drain = tokio::spawn(async move { while receive_control(&mut requests).await.is_ok() {} });
    let _stream = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let event = ControlMessage::Event(HostEvent {
                host_sequence: 1,
                body: ServerEvent::TerminalLeaseRevoked {
                    session_id: TerminalSessionId::new("busy"),
                    previous_owner: ClientInstanceId::new("other"),
                },
            });
            if send_control(&mut host, &event).await.is_err() {
                break;
            }
        }
    });
    let result = tokio::time::timeout(Duration::from_secs(120), task)
        .await
        .expect("an unanswered heartbeat must end the session")
        .unwrap();
    assert!(result.is_some_and(|message| message.contains("heartbeat")));
}

#[test]
fn workspace_and_project_requests_get_longer_on_a_remote_link() {
    let open = Request::Workspace(yttt_protocol::workspace::WorkspaceRequest::List);
    assert_eq!(request_timeout(&open, LinkProfile::LOCAL), REQUEST_TIMEOUT);
    assert_eq!(
        request_timeout(&open, LinkProfile::REMOTE),
        REMOTE_REQUEST_TIMEOUT
    );
    let ping = Request::Ping { sent_millis: 0 };
    assert_eq!(request_timeout(&ping, LinkProfile::REMOTE), REQUEST_TIMEOUT);
}

/// A Host that drops a connection as soon as it is up must not be redialed
/// at the first backoff step forever; only a connection that lasted resets it.
#[test]
fn a_connection_dropped_soon_after_it_came_up_keeps_backing_off() {
    assert_eq!(reconnect_attempt(0, Duration::from_millis(300)), 1);
    assert_eq!(reconnect_attempt(3, Duration::from_millis(300)), 4);
    assert_eq!(reconnect_attempt(3, STABLE_CONNECTION), 1);
    assert_eq!(reconnect_attempt(u32::MAX, Duration::ZERO), u32::MAX);
}

#[test]
fn binding_a_session_waits_longer_on_a_remote_link() {
    assert_eq!(binding_timeout(LinkProfile::LOCAL), REQUEST_TIMEOUT);
    assert!(binding_timeout(LinkProfile::REMOTE) >= LinkProfile::REMOTE.liveness_timeout);
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

fn layout(revision: u64) -> HostWorkspaceState {
    HostWorkspaceState {
        revision,
        workspace: yttt_core::model::workspace::WorkspaceState {
            opened_projects: Vec::new(),
            selected_project_id: None,
        },
        editor: Default::default(),
    }
}

fn layout_changed(revision: u64) -> HostEvent {
    HostEvent {
        host_sequence: revision,
        body: ServerEvent::WorkspaceChanged(Box::new(WorkspaceLayoutUpdate {
            workspace_id: WorkspaceId::new("default").unwrap(),
            layout: layout(revision),
        })),
    }
}

#[test]
fn the_layout_mirror_keeps_the_newest_revision_until_the_host_changes() {
    let context = context();
    let id = WorkspaceId::new("default").unwrap();
    let mut events = context.events.subscribe();
    let (reply, _response) = oneshot::channel();
    handle_response(
        PendingRequest::User {
            reply,
            detaches: Vec::new(),
            opens: Some(id.clone()),
        },
        Ok(Response::Workspace(WorkspaceResponse::Opened {
            layout: layout(2),
            documents: Vec::new(),
            agent_states: Vec::new(),
            agent_summaries: Vec::new(),
        })),
        &context,
    );
    // A change queued before the Open response arrived is older; it is dropped.
    handle_state_event(layout_changed(1), &context).unwrap();
    handle_state_event(layout_changed(3), &context).unwrap();
    handle_state_event(layout_changed(3), &context).unwrap();
    let mut published = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            ClientEvent::WorkspaceLayoutChanged(update) => published.push(update.layout.revision),
            other => panic!("layouts publish only as layout changes: {other:?}"),
        }
    }
    assert_eq!(published, [2, 3]);

    install_host_epoch(
        2,
        &HostEpochState {
            current_host_epoch: &context.current_host_epoch,
            terminal_state: &context.terminal_state,
            catalog: &context.catalog,
            agent_snapshots: &context.agent_snapshots,
            project_token_usage: &context.project_token_usage,
            workspace_layouts: &context.workspace_layouts,
            events: &context.events,
        },
    );
    // The new Host's revisions restart; the workspace stays opened for reopening.
    assert_eq!(context.workspace_layouts.read().get(&id), Some(&None));
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

/// A connector whose every attempt fails in a way worth retrying.
#[derive(Clone)]
struct UnreachableConnector(LinkProfile);

impl TransportConnector for UnreachableConnector {
    fn connect(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        Box::pin(async {
            Err(yttt_transport::TransportError::Other(
                "P2P connection timed out".to_string(),
            ))
        })
    }

    fn link_profile(&self) -> LinkProfile {
        self.0
    }
}

/// A remote window that just started can take well over ten seconds to
/// reach its Host, as a P2P endpoint finds its relay; it keeps trying for
/// as long as one remote connection may take, while a local one gives up
/// at ten seconds.
#[tokio::test(start_paused = true)]
async fn a_remote_connect_keeps_retrying_for_the_links_connect_timeout() {
    for (link, waited) in [
        (LinkProfile::LOCAL, INITIAL_CONNECT_TIMEOUT),
        (LinkProfile::REMOTE, LinkProfile::REMOTE.connect_timeout),
    ] {
        let started = tokio::time::Instant::now();
        let result = ClientCore::connect(
            UnreachableConnector(link),
            identity(),
            yttt_transport::AuthToken::from_bytes([7; 32]),
        )
        .await;
        assert!(matches!(result, Err(ClientCoreError::ConnectionTimeout)));
        assert_eq!(started.elapsed().as_secs(), waited.as_secs(), "{link:?}");
    }
}
