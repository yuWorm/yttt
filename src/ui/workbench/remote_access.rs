use super::*;
use yttt_protocol::{Request, Response, remote_access::*};

/// A section of the remote access page.
pub(super) fn remote_card(theme: WorkbenchTheme, style: UiStyle) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(style.spacing.md)
        .p(gpui::rems(1.25))
        .rounded(style.radius.card)
        .border(style.border.hairline)
        .border_color(theme.border_variant)
        .bg(theme.surface)
}

/// A card's title row, with what it acts on at the right.
pub(super) fn remote_card_header(
    title: &str,
    detail: Option<String>,
    trailing: Option<AnyElement>,
    theme: WorkbenchTheme,
    style: UiStyle,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(style.spacing.lg)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(style.spacing.sm)
                .child(div().text_base().child(title.to_owned()))
                .children(
                    detail.map(|detail| div().text_sm().text_color(theme.text_muted).child(detail)),
                ),
        )
        .children(trailing)
}

impl WorkbenchView {
    /// "Remote access to this computer": what it can do now, the devices
    /// that reach it, how they reach it from outside its network, and the
    /// TLS listener for tunnels, folded away.
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
        let summary = self.remote_summary_card(cx);
        let devices = self.p2p_settings_section(cx);
        let relay_account = self.relay_account_section(window, cx);
        let advanced = self.tls_advanced_card(window, cx);
        div()
            .debug_selector(|| "remote-access-page".into())
            .flex()
            .flex_col()
            .w_full()
            .max_w(gpui::rems(46.0))
            .mx_auto()
            .gap(gpui::rems(1.0))
            .when_some(self.ssh.remote_access_error.clone(), |view, message| {
                view.child(
                    div()
                        .debug_selector(|| "remote-access-notice".into())
                        .px(gpui::rems(1.0))
                        .py(style.spacing.sm)
                        .rounded(style.radius.card)
                        .bg(theme.element_background)
                        .text_sm()
                        .child(message),
                )
            })
            .child(summary)
            .child(devices)
            .child(relay_account)
            .child(advanced)
    }

    /// Whether remote access can be changed now: a local Host answers and no
    /// other change is under way.
    pub(super) fn remote_access_idle(&self) -> bool {
        !self.ssh.remote_access_busy
            && (self.ssh.preview
                || self
                    .terminal
                    .host_runtime
                    .as_ref()
                    .is_some_and(|runtime| !runtime.is_remote()))
    }

    /// Whether this computer accepts paired devices and how they reach it,
    /// with the one switch that turns it on.
    fn remote_summary_card(&mut self, cx: &mut Context<Self>) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let idle = self.remote_access_idle();
        let status = self.ssh.remote_access.as_ref();
        let p2p = status.map(|status| &status.p2p);
        let effective = p2p.map(|p2p| p2p.effective);
        let listening = effective == Some(RemoteAccessState::Listening);
        let pairing_active = self.ssh.p2p.code.is_some()
            || matches!(
                p2p.and_then(|p2p| p2p.pairing.as_ref()),
                Some(PairingState::AwaitingConfirmation { .. })
            );
        let (headline, dot) = match effective {
            Some(RemoteAccessState::Listening) => (UiTextKey::RemoteSummaryOn, theme.success),
            Some(RemoteAccessState::Starting) => (UiTextKey::RemoteAccessStarting, theme.warning),
            Some(RemoteAccessState::Stopping) => (UiTextKey::RemoteAccessStopping, theme.warning),
            Some(RemoteAccessState::Failed) => (UiTextKey::RemoteAccessFailed, theme.danger),
            Some(RemoteAccessState::Disabled) => (UiTextKey::RemoteSummaryOff, theme.text_subtle),
            None => (UiTextKey::RemoteAccessUnavailable, theme.text_subtle),
        };
        let paired = self.ssh.p2p.devices.len();
        let online = status.map_or(0, |status| {
            self.ssh
                .p2p
                .devices
                .iter()
                .filter(|device| {
                    status
                        .clients
                        .iter()
                        .any(|client| client.device == Some(device.endpoint_id))
                })
                .count()
        });
        let relay = p2p.and_then(|p2p| p2p.relay_account.as_ref());
        let node_name = |id: &str| {
            self.ssh
                .relay_account
                .nodes
                .as_ref()
                .and_then(|nodes| nodes.iter().find(|node| node.id == id))
                .map_or_else(|| id.to_owned(), |node| node.name.clone())
        };
        let (route, route_color) = match relay {
            Some(account) if account.placement_needed.is_some() => (
                text.get(UiTextKey::RemoteSummaryChooseNode).to_owned(),
                theme.warning,
            ),
            Some(account) => match &account.node_id {
                Some(node) => (
                    text.get(UiTextKey::RemoteSummaryRelay)
                        .replace("{node}", &node_name(node)),
                    theme.text_muted,
                ),
                None => (
                    text.get(UiTextKey::RemoteSummaryRelayPending).to_owned(),
                    theme.text_muted,
                ),
            },
            None => (
                text.get(UiTextKey::RemoteSummaryDirectOnly).to_owned(),
                theme.text_muted,
            ),
        };
        let toggle = yttt_switch("remote-p2p-switch", listening, theme, style, {
            let owner = cx.weak_entity();
            move |checked, _, cx| {
                let checked = *checked;
                let _ = owner.update(cx, |root, cx| root.set_p2p(checked, cx));
            }
        });
        remote_card(theme, style)
            .debug_selector(|| "remote-summary".into())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(style.spacing.lg)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(style.spacing.xs)
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(style.spacing.sm)
                                    .child(div().size(gpui::rems(0.5)).rounded_full().bg(dot))
                                    .child(div().text_base().child(text.get(headline))),
                            )
                            .when(listening, |view| {
                                view.child(
                                    div().text_sm().text_color(theme.text_muted).child(
                                        text.get(UiTextKey::RemoteSummaryDevices)
                                            .replace("{paired}", &paired.to_string())
                                            .replace("{online}", &online.to_string()),
                                    ),
                                )
                            })
                            .when(listening, |view| {
                                view.child(div().text_sm().text_color(route_color).child(route))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(style.spacing.sm)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.text_muted)
                                    .child(text.get(UiTextKey::RemoteSummaryToggle)),
                            )
                            .child(if idle && !pairing_active {
                                toggle
                            } else {
                                toggle.opacity(0.5).occlude()
                            }),
                    ),
            )
            .when(!listening, |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::P2pDescription)),
                )
            })
            .when_some(p2p.and_then(|p2p| p2p.error.clone()), |view, error| {
                view.child(div().text_sm().text_color(theme.danger).child(error))
            })
    }

    /// The TLS listener for port forwarding and tunnels, folded away unless
    /// it is on.
    fn tls_advanced_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let busy = !self.remote_access_idle();
        let status = self.ssh.remote_access.as_ref();
        let enabled = status.is_some_and(|status| status.effective == RemoteAccessState::Listening);
        let expanded = self.ssh.tls_expanded || enabled;
        let state_line = match status.map(|status| status.effective) {
            Some(RemoteAccessState::Listening) => {
                status.and_then(|status| status.bound_address).map_or_else(
                    || text.get(UiTextKey::RemoteAccessListening).to_owned(),
                    |address| format!("{} {address}", text.get(UiTextKey::RemoteAccessListening)),
                )
            }
            Some(RemoteAccessState::Starting) => text.get(UiTextKey::RemoteAccessStarting).into(),
            Some(RemoteAccessState::Stopping) => text.get(UiTextKey::RemoteAccessStopping).into(),
            Some(RemoteAccessState::Failed) => text.get(UiTextKey::RemoteAccessFailed).into(),
            _ => text.get(UiTextKey::RemoteTlsOff).into(),
        };
        let header = div()
            .id("remote-tls-toggle-section")
            .debug_selector(|| "remote-tls-header".into())
            .flex()
            .items_center()
            .gap(style.spacing.sm)
            .cursor_pointer()
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.0))
                .text_color(theme.text_subtle),
            )
            .child(div().text_sm().child(text.get(UiTextKey::RemoteTlsTitle)))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(state_line),
            )
            .on_click(cx.listener(move |root, _, _, cx| {
                root.ssh.tls_expanded = !expanded;
                cx.notify();
            }));
        let card = remote_card(theme, style)
            .debug_selector(|| "remote-tls".into())
            .child(header);
        if !expanded {
            return card;
        }
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
        ] {
            actions = actions.child(
                yttt_button(
                    id,
                    text.get(key),
                    YtttButtonVariant::Secondary,
                    theme,
                    style,
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
        // Sessions that came over TLS; paired devices show in their card.
        let clients: Vec<_> = status
            .map(|status| {
                status
                    .clients
                    .iter()
                    .filter(|client| client.device.is_none())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let any_session = status.is_some_and(|status| !status.clients.is_empty());
        card.child(
            div()
                .text_sm()
                .text_color(theme.text_muted)
                .child(text.get(UiTextKey::RemoteAccessDescription)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(style.spacing.sm)
                .child(
                    yttt_input(&input, YtttInputKind::Settings, theme, style)
                        .disabled(busy)
                        .w(gpui::rems(16.0)),
                )
                .child(
                    yttt_button(
                        "remote-access-toggle",
                        text.get(if enabled {
                            UiTextKey::RemoteAccessDisable
                        } else {
                            UiTextKey::RemoteTlsEnable
                        }),
                        if enabled {
                            YtttButtonVariant::Secondary
                        } else {
                            YtttButtonVariant::Primary
                        },
                        theme,
                        style,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(move |root, _, window, cx| {
                        root.set_remote_access_enabled(!enabled, window, cx)
                    })),
                )
                .when(enabled, |row| {
                    row.child(
                        yttt_button(
                            "remote-access-address",
                            text.get(UiTextKey::RemoteAccessApply),
                            YtttButtonVariant::Ghost,
                            theme,
                            style,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|root, _, window, cx| {
                            root.set_remote_access_enabled(true, window, cx)
                        })),
                    )
                }),
        )
        .when(enabled, |card| card.child(actions))
        .when(enabled, |card| {
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.xs)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::RemoteTlsSessions)),
                    )
                    .when(clients.is_empty(), |view| {
                        view.child(
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
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(client.client_id.to_string()),
                            )
                            .child(div().text_xs().text_color(theme.text_muted).child(format!(
                                "{} {}",
                                client.streams,
                                text.get(UiTextKey::RemoteAccessStreams)
                            )))
                    })),
            )
        })
        .when(any_session, |card| {
            card.child(
                div().child(
                    yttt_button(
                        "remote-access-disconnect",
                        text.get(UiTextKey::RemoteAccessDisconnect),
                        YtttButtonVariant::Ghost,
                        theme,
                        style,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|root, _, window, cx| {
                        root.remote_access_action(
                            Request::RemoteAccess(RemoteAccessRequest::DisconnectAll),
                            true,
                            window,
                            cx,
                        );
                    })),
                ),
            )
        })
        .when_some(
            status.and_then(|status| status.error.clone()),
            |card, error| card.child(div().text_sm().text_color(theme.danger).child(error)),
        )
    }

    fn set_remote_access_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal.host_runtime.is_none() {
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
                self.ssh.remote_access_error = Some(format!(
                    "{}: {error}",
                    self.ui_text
                        .get(UiTextKey::WorkbenchI18nRemoteAccessInvalidAddress)
                ));
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
        window.open_alert_dialog(cx, move |alert, _, _| {
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

    pub(super) fn begin_remote_access_action(&mut self, request: Request, cx: &mut Context<Self>) {
        self.begin_remote_access_actions(vec![request], cx);
    }

    /// Sends `requests` to the Host one after another, stopping at the first
    /// that fails, then shows the Host's status; the last answer is handled
    /// as `begin_remote_access_action` handles its one.
    pub(super) fn begin_remote_access_actions(
        &mut self,
        requests: Vec<Request>,
        cx: &mut Context<Self>,
    ) {
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
            let mut result = Err(String::new());
            for request in requests {
                result = runtime.request_blocking(request);
                if result.is_err() {
                    break;
                }
            }
            let status =
                runtime.request_blocking(Request::RemoteAccess(RemoteAccessRequest::Status));
            (result, status)
        });
        cx.spawn(async move |this, cx| {
            let (result, status) = task.await;
            let _ = this.update(cx, |root, cx| {
                root.ssh.remote_access_busy = false;
                root.ssh.remote_access_loaded = true;
                if let Ok(Response::RemoteAccess(response)) = &result {
                    root.p2p_action_result(response, cx);
                }
                if let Ok(Response::RemoteAccess(RemoteAccessResponse::Status(status))) = status {
                    root.p2p_action_result(&RemoteAccessResponse::Status(status.clone()), cx);
                    root.ssh.remote_access = Some(status);
                }
                match result {
                    Ok(Response::RemoteAccess(RemoteAccessResponse::ConnectionInfo(info))) => {
                        let payload = root
                            .ssh
                            .remote_access
                            .as_ref()
                            .and_then(|status| status.bound_address)
                            .map(crate::remote_launch::exported_address)
                            .ok_or_else(|| {
                                root.ui_text
                                    .get(UiTextKey::WorkbenchI18nRemoteAccessNotListening)
                                    .to_string()
                            })
                            .and_then(|address| {
                                crate::remote_launch::ConnectionCode::encode(
                                    address.to_string(),
                                    info,
                                )
                                .map(|payload| (address, payload))
                                .map_err(str::to_string)
                            });
                        match payload {
                            Ok((address, payload)) => {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(payload));
                                let text = &root.ui_text;
                                let mut message = format!(
                                    "{} {} {address}",
                                    text.get(UiTextKey::RemoteAccessCopied),
                                    text.get(UiTextKey::RemoteAccessCodeAddress),
                                );
                                if address.ip().is_loopback() {
                                    message.push(' ');
                                    message.push_str(text.get(UiTextKey::RemoteAccessLoopbackOnly));
                                }
                                root.ssh.remote_access_error = Some(message);
                            }
                            Err(error) => {
                                root.ssh.remote_access_error = Some(format!(
                                    "{}: {error}",
                                    root.ui_text
                                        .get(UiTextKey::WorkbenchI18nRemoteAccessActionFailed)
                                ));
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        root.ssh.remote_access_error = Some(format!(
                            "{}: {error}",
                            root.ui_text
                                .get(UiTextKey::WorkbenchI18nRemoteAccessActionFailed)
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
