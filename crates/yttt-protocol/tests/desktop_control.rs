use yttt_protocol::{desktop_control::*, *};

fn roundtrip(message: DesktopShellMessage) {
    let bytes = encode_message(FrameKind::DesktopShell, &message).unwrap();
    let decoded: DesktopShellMessage = decode_message(&decode_frame(&bytes).unwrap()).unwrap();
    assert_eq!(message, decoded);
}

#[test]
fn control_request_and_agent_response_survive_wire_roundtrip() {
    roundtrip(DesktopShellMessage::Request(DesktopShellRequestEnvelope {
        protocol_version: DESKTOP_SHELL_PROTOCOL_VERSION,
        profile_id: yttt_core::model::ids::ProfileId::new("test"),
        request_id: 1,
        body: DesktopShellRequest::Control(Box::new(DesktopControlRequest {
            window: Some("1".into()),
            project: Some("p".into()),
            tab: Some("t".into()),
            pane: Some("shell".into()),
            command: DesktopControlCommand::Send {
                text: "line 1\n第二行".into(),
                enter: true,
                raw: false,
                agent_only: true,
            },
        })),
    }));
    let snapshot = yttt_agent_core::AgentReducer::new(
        yttt_agent_core::AgentInstanceId::new("agent").unwrap(),
        yttt_agent_core::ProviderId::from_static("codex"),
        1,
    )
    .snapshot()
    .clone();
    roundtrip(DesktopShellMessage::Response(
        DesktopShellResponseEnvelope {
            request_id: 1,
            result: DesktopShellResponse::Control(Box::new(Ok(DesktopControlResponse::Agents(
                vec![ControlPane {
                    target: ControlTarget {
                        window: "1".into(),
                        project: "p".into(),
                        tab: "t".into(),
                        pane: Some("a".into()),
                    },
                    title: "Agent".into(),
                    command: "codex".into(),
                    session_id: "p:t:a".into(),
                    state: "starting".into(),
                    focused: true,
                    agent: Some(snapshot),
                }],
            )))),
        },
    ));
}

#[test]
fn control_validation_rejects_invalid_targets_and_payloads() {
    let mut request = DesktopControlRequest {
        window: None,
        project: Some("p".into()),
        tab: Some("t".into()),
        pane: Some("shell".into()),
        command: DesktopControlCommand::Send {
            text: "ok".into(),
            enter: true,
            raw: false,
            agent_only: false,
        },
    };
    assert!(request.validate().is_ok());
    request.tab = None;
    assert!(request.validate().is_err());
    request.tab = Some("t".into());
    request.command = DesktopControlCommand::Rename {
        title: "\u{1b}[2J".into(),
    };
    assert!(request.validate().is_err());
    request.command = DesktopControlCommand::Send {
        text: "x".repeat(MAX_CONTROL_TEXT_BYTES + 1),
        enter: false,
        raw: false,
        agent_only: false,
    };
    assert!(request.validate().is_err());
}
