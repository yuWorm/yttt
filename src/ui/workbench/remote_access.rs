use super::*;
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
        let runtime = self
            .terminal
            .host_runtime
            .clone()
            .filter(|runtime| !runtime.is_remote());
        if runtime.is_some() && !self.ssh.remote_access_loaded && !self.ssh.remote_access_busy {
            self.begin_remote_access_action(Request::RemoteAccess(RemoteAccessRequest::Status), cx);
        }
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let busy = self.ssh.remote_access_busy || runtime.is_none();
        let status = self.ssh.remote_access.as_ref();
        let enabled = status.is_some_and(|status| status.effective == RemoteAccessState::Listening);
        let state_key = match status.map(|status| status.effective) {
            Some(RemoteAccessState::Listening) => UiTextKey::RemoteAccessListening,
            Some(RemoteAccessState::Starting) => UiTextKey::RemoteAccessStarting,
            Some(RemoteAccessState::Stopping) => UiTextKey::RemoteAccessStopping,
            Some(RemoteAccessState::Failed) => UiTextKey::RemoteAccessFailed,
            Some(RemoteAccessState::Disabled) => UiTextKey::RemoteAccessDisabled,
            None => UiTextKey::RemoteAccessUnavailable,
        };
        let initial_address = status
            .map(|status| status.settings.listen_address.to_string())
            .unwrap_or_else(|| DEFAULT_REMOTE_ADDRESS.into());
        let input = self
            .ssh
            .remote_access_address
            .get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).default_value(initial_address))
            })
            .clone();
        let mut connection_actions = div().flex().flex_wrap().gap(style.spacing.sm);
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
        ] {
            connection_actions = connection_actions.child(
                yttt_button(
                    id,
                    text.get(key),
                    YtttButtonVariant::Secondary,
                    theme,
                    style,
                    cx,
                )
                .disabled(busy || !enabled)
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
            .as_ref()
            .and_then(|runtime| runtime.control_status())
            .and_then(|status| status.transfer)
            .filter(|transfer| transfer.phase == TransferPhase::ForceConfirmationRequired)
            .map(|transfer| ProfileControlRequest::ConfirmForce {
                transfer_id: transfer.id,
            })
            .unwrap_or(ProfileControlRequest::RequestControl);
        let force = matches!(reclaim, ProfileControlRequest::ConfirmForce { .. });
        let clients = status
            .map(|status| status.clients.as_slice())
            .unwrap_or_default();
        div()
            .debug_selector(|| "remote-access-page".into())
            .flex()
            .flex_col()
            .w_full()
            .max_w(gpui::rems(48.0))
            .gap(gpui::rems(1.5))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.sm)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(style.spacing.lg)
                            .child(
                                div()
                                    .text_base()
                                    .child(text.get(UiTextKey::RemoteAccessTitle)),
                            )
                            .child(
                                yttt_button(
                                    "remote-access-toggle",
                                    text.get(if enabled {
                                        UiTextKey::RemoteAccessDisable
                                    } else {
                                        UiTextKey::RemoteAccessEnable
                                    }),
                                    if enabled {
                                        YtttButtonVariant::Secondary
                                    } else {
                                        YtttButtonVariant::Primary
                                    },
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
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::RemoteAccessDescription)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py(style.spacing.sm)
                    .border_b(style.border.hairline)
                    .border_color(theme.border_variant)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(style.spacing.sm)
                            .child(div().size(gpui::rems(0.375)).rounded_full().bg(if enabled {
                                theme.success
                            } else {
                                theme.text_subtle
                            }))
                            .child(div().text_sm().child(text.get(state_key)))
                            .when_some(
                                status.and_then(|status| status.bound_address),
                                |this, address| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_muted)
                                            .child(address.to_string()),
                                    )
                                },
                            ),
                    )
                    .child(
                        yttt_button(
                            "remote-access-refresh",
                            text.get(UiTextKey::RemoteAccessRefresh),
                            YtttButtonVariant::Ghost,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.begin_remote_access_action(
                                Request::RemoteAccess(RemoteAccessRequest::Status),
                                cx,
                            );
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.md)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::RemoteAccessAddress)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(style.spacing.sm)
                            .child(
                                yttt_input(&input, YtttInputKind::Settings, theme, style)
                                    .disabled(busy)
                                    .w(gpui::rems(18.0)),
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
                                .on_click(cx.listener(
                                    |root, _, window, cx| {
                                        root.set_remote_access_enabled(true, window, cx)
                                    },
                                )),
                            ),
                    )
                    .child(connection_actions),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.md)
                    .pt(style.spacing.lg)
                    .border_t(style.border.hairline)
                    .border_color(theme.border_variant)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(text.get(UiTextKey::RemoteAccessClients)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(clients.len().to_string()),
                            ),
                    )
                    .when(clients.is_empty(), |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child(text.get(UiTextKey::RemoteAccessNoClients)),
                        )
                    })
                    .children(clients.iter().map(|client| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(style.spacing.lg)
                            .child(div().text_sm().child(client.client_id.to_string()))
                            .child(div().text_xs().text_color(theme.text_muted).child(format!(
                                "{} {}",
                                client.streams,
                                text.get(UiTextKey::RemoteAccessStreams)
                            )))
                    }))
                    .when_some(
                        status.and_then(|status| status.control.owner.as_ref()),
                        |this, owner| {
                            this.child(div().text_xs().text_color(theme.text_muted).child(format!(
                                "{}: {owner}",
                                text.get(UiTextKey::RemoteAccessControlOwner)
                            )))
                        },
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(style.spacing.sm)
                            .child(
                                yttt_button(
                                    "remote-access-disconnect",
                                    text.get(UiTextKey::RemoteAccessDisconnect),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(busy || clients.is_empty())
                                .on_click(cx.listener(
                                    |root, _, window, cx| {
                                        root.remote_access_action(
                                            Request::RemoteAccess(
                                                RemoteAccessRequest::DisconnectAll,
                                            ),
                                            true,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                yttt_button(
                                    "remote-access-reclaim",
                                    text.get(UiTextKey::RemoteAccessReclaim),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(
                                    busy || runtime
                                        .as_ref()
                                        .is_some_and(|runtime| runtime.is_controller()),
                                )
                                .on_click(cx.listener(
                                    move |root, _, window, cx| {
                                        root.remote_access_action(
                                            Request::ProfileControl(reclaim.clone()),
                                            force,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    ),
            )
            .when_some(
                status.and_then(|status| status.error.as_ref()),
                |view, error| {
                    view.child(
                        div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(error.clone()),
                    )
                },
            )
            .when_some(self.ssh.remote_access_error.clone(), |view, message| {
                view.child(div().text_sm().text_color(theme.text_muted).child(message))
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
            self.ssh.remote_access_error = Some(
                self.ui_text
                    .get(UiTextKey::RemoteAccessNotReady)
                    .to_string(),
            );
            cx.notify();
            return;
        }
        let address = self
            .ssh
            .remote_access_address
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let address = match address.trim().parse::<std::net::SocketAddr>() {
            Ok(address) => address,
            Err(error) => {
                self.ssh.remote_access_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let confirm =
            !enabled
                || !address.ip().is_loopback()
                || self.ssh.remote_access.as_ref().is_some_and(|status| {
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
        if self.ssh.remote_access_busy {
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
        if self.ssh.remote_access_busy {
            return;
        }
        self.ssh.remote_access_busy = true;
        self.ssh.remote_access_error = None;
        let task = cx.background_spawn(async move {
            let result = runtime.request_blocking(request);
            let status =
                runtime.request_blocking(Request::RemoteAccess(RemoteAccessRequest::Status));
            (result, status)
        });
        cx.spawn(async move |this, cx| {
            let (result, status) = task.await;
            let _ = this.update(cx, |root, cx| {
                root.ssh.remote_access_busy = false;
                root.ssh.remote_access_loaded = true;
                if let Ok(Response::RemoteAccess(RemoteAccessResponse::Status(status))) = status {
                    root.ssh.remote_access = Some(status);
                }
                match result {
                    Ok(Response::RemoteAccess(RemoteAccessResponse::ConnectionInfo(info))) => {
                        let payload = root
                            .ssh
                            .remote_access
                            .as_ref()
                            .and_then(|status| status.bound_address)
                            .ok_or("Remote access is not listening")
                            .and_then(|address| {
                                crate::remote_launch::ConnectionCode::encode(
                                    address.to_string(),
                                    info,
                                )
                            });
                        match payload {
                            Ok(payload) => {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(payload));
                                root.ssh.remote_access_error = Some(
                                    root.ui_text.get(UiTextKey::RemoteAccessCopied).to_string(),
                                );
                            }
                            Err(error) => root.ssh.remote_access_error = Some(error.to_string()),
                        }
                    }
                    Ok(_) => {}
                    Err(error) => root.ssh.remote_access_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
