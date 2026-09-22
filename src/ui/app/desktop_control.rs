use super::*;
use yttt_protocol::desktop_control::*;

pub(super) fn start_desktop_control_listener(desktop: Arc<DesktopShellRuntime>, cx: &mut App) {
    let calls = desktop.controls();
    cx.spawn(async move |cx| {
        let _desktop = desktop;
        while let Ok(call) = calls.recv_async().await {
            cx.update(|cx| route_control(call, cx));
        }
    })
    .detach();
}

fn route_control(call: crate::desktop_shell::DesktopControlCall, cx: &mut App) {
    let reject = |code, message: &str| {
        let _ = call
            .reply
            .send(Err(DesktopControlError::new(code, message)));
    };
    if std::time::Instant::now() >= call.deadline || call.reply.is_disconnected() {
        reject(
            DesktopControlErrorCode::NotReady,
            "Request expired before execution",
        );
        return;
    }
    if let Err(error) = call.request.validate() {
        let _ = call.reply.send(Err(error));
        return;
    }
    let mut windows = Vec::new();
    for handle in cx.windows() {
        let view = handle
            .update(cx, |_, window, cx| {
                window.root::<ComponentRoot>().flatten().and_then(|root| {
                    root.read(cx)
                        .view()
                        .clone()
                        .downcast::<WorkbenchView>()
                        .ok()
                })
            })
            .ok()
            .flatten();
        if let Some(view) = view {
            let id = view.entity_id().as_u64().to_string();
            if call
                .request
                .window
                .as_ref()
                .is_none_or(|wanted| wanted == &id)
                && view.read(cx).control_matches(&call.request)
            {
                windows.push((handle, view, id));
            }
        }
    }
    if call.request.command.is_list() {
        let mut result = match call.request.command {
            DesktopControlCommand::Windows => DesktopControlResponse::Windows(Vec::new()),
            DesktopControlCommand::Projects => DesktopControlResponse::Projects(Vec::new()),
            DesktopControlCommand::Tabs => DesktopControlResponse::Tabs(Vec::new()),
            DesktopControlCommand::Agents => DesktopControlResponse::Agents(Vec::new()),
            _ => DesktopControlResponse::Panes(Vec::new()),
        };
        if windows.is_empty() && (call.request.window.is_some() || call.request.project.is_some()) {
            reject(
                DesktopControlErrorCode::NotFound,
                "No open window matches the requested target",
            );
            return;
        }
        for (_, view, id) in windows {
            let next = view.read(cx).control_list(&id, &call.request, cx);
            match (&mut result, next) {
                (
                    DesktopControlResponse::Windows(all),
                    DesktopControlResponse::Windows(mut items),
                ) => all.append(&mut items),
                (
                    DesktopControlResponse::Projects(all),
                    DesktopControlResponse::Projects(mut items),
                ) => all.append(&mut items),
                (DesktopControlResponse::Tabs(all), DesktopControlResponse::Tabs(mut items)) => {
                    all.append(&mut items)
                }
                (DesktopControlResponse::Panes(all), DesktopControlResponse::Panes(mut items))
                | (
                    DesktopControlResponse::Agents(all),
                    DesktopControlResponse::Agents(mut items),
                ) => all.append(&mut items),
                _ => unreachable!("list response matches its request"),
            }
        }
        let _ = call.reply.send(Ok(result));
        return;
    }
    if windows.len() > 1 {
        reject(
            DesktopControlErrorCode::AmbiguousTarget,
            "Project is open in multiple windows; specify --window from windows list",
        );
        return;
    }
    let Some((handle, view, id)) = windows.pop() else {
        reject(
            DesktopControlErrorCode::NotFound,
            "No open window contains the requested project",
        );
        return;
    };
    let fallback = call.reply.clone();
    if handle
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.handle_control(&id, call.request, call.reply, window, cx)
            });
        })
        .is_err()
    {
        let _ = fallback.send(Err(DesktopControlError::new(
            DesktopControlErrorCode::NotFound,
            "Target window closed",
        )));
    }
}
