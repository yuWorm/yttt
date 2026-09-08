use super::super::*;
use yttt_protocol::{
    Request, Response,
    remote_access::*,
    session::{ProfileControlRequest, TransferPhase},
};

impl WorkbenchView {
    pub(super) fn remote_access_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(runtime) = self
            .terminal
            .host_runtime
            .clone()
            .filter(|runtime| !runtime.is_remote())
        else {
            return div();
        };
        if !self.settings.remote_access_loaded && !self.settings.remote_access_busy {
            self.begin_remote_access_action(Request::RemoteAccess(RemoteAccessRequest::Status), cx);
        }
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let busy = self.settings.remote_access_busy;
        let status = self.settings.remote_access.as_ref();
        let enabled = status.is_some_and(|status| status.effective == RemoteAccessState::Listening);
        let detail = status
            .map(|status| {
                format!(
                    "{:?} · {} · {} clients · control: {} · epoch {}{}",
                    status.effective,
                    status
                        .bound_address
                        .map(|address| address.to_string())
                        .unwrap_or_else(|| "—".into()),
                    status.clients.len(),
                    status
                        .control
                        .owner
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "—".into()),
                    status.control.context.control_epoch,
                    status
                        .error
                        .as_ref()
                        .map(|error| format!("\n{error}"))
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| "…".into());
        let initial_address = status
            .map(|status| status.settings.listen_address.to_string())
            .unwrap_or_else(|| DEFAULT_REMOTE_ADDRESS.into());
        let input = self
            .settings
            .remote_access_address
            .get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).default_value(initial_address))
            })
            .clone();
        let mut actions = div().flex().flex_wrap().gap(style.spacing.sm);
        for (id, key, request, confirmation) in [
            (
                "remote-access-copy",
                UiTextKey::RemoteAccessCopy,
                RemoteAccessRequest::ExportConnectionInfo,
                false,
            ),
            (
                "remote-access-reset",
                UiTextKey::RemoteAccessReset,
                RemoteAccessRequest::ResetCredentials,
                true,
            ),
            (
                "remote-access-disconnect",
                UiTextKey::RemoteAccessDisconnect,
                RemoteAccessRequest::DisconnectAll,
                true,
            ),
            (
                "remote-access-refresh",
                UiTextKey::RemoteAccessRefresh,
                RemoteAccessRequest::Status,
                false,
            ),
        ] {
            actions = actions.child(
                yttt_button(
                    id,
                    text.get(key),
                    YtttButtonVariant::Secondary,
                    theme,
                    style,
                    cx,
                )
                .disabled(busy || (!enabled && !matches!(request, RemoteAccessRequest::Status)))
                .on_click(cx.listener(move |root, _, window, cx| {
                    root.remote_access_action(
                        Request::RemoteAccess(request.clone()),
                        confirmation,
                        window,
                        cx,
                    );
                })),
            );
        }
        let reclaim = runtime
            .control_status()
            .and_then(|status| status.transfer)
            .filter(|transfer| transfer.phase == TransferPhase::ForceConfirmationRequired)
            .map(|transfer| ProfileControlRequest::ConfirmForce {
                transfer_id: transfer.id,
            })
            .unwrap_or(ProfileControlRequest::RequestControl);
        let force = matches!(reclaim, ProfileControlRequest::ConfirmForce { .. });
        actions = actions.child(
            yttt_button(
                "remote-access-reclaim",
                text.get(UiTextKey::RemoteAccessReclaim),
                YtttButtonVariant::Secondary,
                theme,
                style,
                cx,
            )
            .disabled(busy || runtime.is_controller())
            .on_click(cx.listener(move |root, _, window, cx| {
                root.remote_access_action(
                    Request::ProfileControl(reclaim.clone()),
                    force,
                    window,
                    cx,
                )
            })),
        );
        div()
            .flex()
            .flex_col()
            .gap(style.spacing.sm)
            .py(style.spacing.md)
            .debug_selector(|| "settings-remote-access".to_string())
            .child(
                div()
                    .text_color(theme.text)
                    .child(text.get(UiTextKey::RemoteAccessTitle)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::RemoteAccessDescription)),
            )
            .child(div().text_sm().child(detail))
            .children(
                status
                    .into_iter()
                    .flat_map(|status| status.clients.iter())
                    .map(|client| {
                        div()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(format!("{} · {} streams", client.client_id, client.streams))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(style.spacing.sm)
                    .child(
                        yttt_input(&input, YtttInputKind::Settings, theme, style)
                            .disabled(busy)
                            .w(px(240.0)),
                    )
                    .child(
                        yttt_button(
                            "remote-access-toggle",
                            text.get(if enabled {
                                UiTextKey::RemoteAccessDisable
                            } else {
                                UiTextKey::RemoteAccessEnable
                            }),
                            YtttButtonVariant::Primary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(
                            move |root, _, window, cx| {
                                root.set_remote_access_enabled(!enabled, window, cx)
                            },
                        )),
                    )
                    .child(
                        yttt_button(
                            "remote-access-address",
                            text.get(UiTextKey::RemoteAccessApply),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(busy || !enabled)
                        .on_click(cx.listener(|root, _, window, cx| {
                            root.set_remote_access_enabled(true, window, cx)
                        })),
                    ),
            )
            .child(actions)
            .when_some(self.settings.remote_access_error.clone(), |view, error| {
                view.child(div().text_sm().child(error))
            })
    }

    fn set_remote_access_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = &self.terminal.host_runtime else {
            return;
        };
        if enabled && !runtime.sharing_ready() {
            self.settings.remote_access_error = Some(
                self.ui_text
                    .get(UiTextKey::RemoteAccessNotReady)
                    .to_string(),
            );
            cx.notify();
            return;
        }
        let address = self
            .settings
            .remote_access_address
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let address = match address.trim().parse::<std::net::SocketAddr>() {
            Ok(address) => address,
            Err(error) => {
                self.settings.remote_access_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let confirm =
            !enabled
                || !address.ip().is_loopback()
                || self.settings.remote_access.as_ref().is_some_and(|status| {
                    status.bound_address.is_some_and(|bound| bound != address)
                });
        self.remote_access_action(
            Request::RemoteAccess(RemoteAccessRequest::SetEnabled {
                enabled,
                listen_address: address,
                confirm_non_loopback: confirm,
            }),
            confirm,
            window,
            cx,
        );
    }

    fn remote_access_action(
        &mut self,
        request: Request,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.remote_access_busy {
            return;
        }
        if !confirm {
            self.begin_remote_access_action(request, cx);
            return;
        }
        let appearance = self.theme_runtime();
        let text = self.ui_text;
        let root = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let root = root.clone();
            let request = request.clone();
            alert
                .title(text.get(UiTextKey::RemoteAccessConfirmTitle))
                .description(text.get(UiTextKey::RemoteAccessConfirmDescription))
                .footer(
                    DialogFooter::new()
                        .child(
                            yttt_button(
                                "remote-access-cancel",
                                text.get(UiTextKey::Cancel),
                                YtttButtonVariant::Secondary,
                                appearance.ui,
                                appearance.style,
                                cx,
                            )
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            yttt_button(
                                "remote-access-confirm",
                                text.get(UiTextKey::RemoteAccessConfirm),
                                YtttButtonVariant::Primary,
                                appearance.ui,
                                appearance.style,
                                cx,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = root.update(cx, |root, cx| {
                                    root.begin_remote_access_action(request.clone(), cx)
                                });
                                window.close_dialog(cx);
                            }),
                        ),
                )
        });
    }

    fn begin_remote_access_action(&mut self, request: Request, cx: &mut Context<Self>) {
        let Some(runtime) = self
            .terminal
            .host_runtime
            .clone()
            .filter(|runtime| !runtime.is_remote())
        else {
            return;
        };
        if self.settings.remote_access_busy {
            return;
        }
        self.settings.remote_access_busy = true;
        self.settings.remote_access_error = None;
        let task = cx.background_spawn(async move {
            let result = runtime.request_blocking(request);
            let status =
                runtime.request_blocking(Request::RemoteAccess(RemoteAccessRequest::Status));
            (result, status)
        });
        cx.spawn(async move |this, cx| {
            let (result, status) = task.await;
            let _ = this.update(cx, |root, cx| {
                root.settings.remote_access_busy = false;
                root.settings.remote_access_loaded = true;
                if let Ok(Response::RemoteAccess(RemoteAccessResponse::Status(status))) = status {
                    root.settings.remote_access = Some(status);
                }
                match result {
                    Ok(Response::RemoteAccess(RemoteAccessResponse::ConnectionInfo(info))) => {
                        match serde_json::to_string(&info) {
                            Ok(payload) if payload.len() <= MAX_CONNECTION_INFO_BYTES => {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(payload));
                                root.settings.remote_access_error = Some(
                                    root.ui_text.get(UiTextKey::RemoteAccessCopied).to_string(),
                                );
                            }
                            Ok(_) => {
                                root.settings.remote_access_error =
                                    Some("Connection information exceeds 8 KiB".into())
                            }
                            Err(error) => {
                                root.settings.remote_access_error = Some(error.to_string())
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(error) => root.settings.remote_access_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
