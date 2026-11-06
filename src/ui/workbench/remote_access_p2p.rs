//! The P2P part of "Remote access to this computer": turning P2P on, pairing a
//! device by code, and the devices paired so far.
//!
//! Only the local runtime is ever asked; a remote Host's settings never show
//! here, so nothing reached over the network can change who is paired.

use std::{sync::Arc, time::Duration};

use super::*;
use yttt_protocol::{
    EndpointKey, Request, Response,
    p2p::{DevicePreset, PairedDevice},
    remote_access::*,
};

/// How often the page asks the Host about an open invitation.
const PAIRING_POLL: Duration = Duration::from_secs(1);

/// A QR code's modules, row by row; `true` is dark.
pub(super) struct QrMatrix {
    width: usize,
    dark: Vec<bool>,
}

impl QrMatrix {
    pub(super) fn encode(text: &str) -> Option<Self> {
        let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
            .ok()?;
        let width = code.width();
        let dark = code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();
        Some(Self { width, dark })
    }
}

impl WorkbenchView {
    fn local_runtime(&self) -> Option<Arc<crate::host_runtime::DesktopHostRuntime>> {
        self.terminal
            .host_runtime
            .clone()
            .filter(|runtime| !runtime.is_remote())
    }

    pub(super) fn p2p_settings_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let busy = self.ssh.remote_access_busy || self.local_runtime().is_none();
        let status = self
            .ssh
            .remote_access
            .as_ref()
            .map(|status| status.p2p.clone());
        let settings = self
            .ssh
            .remote_access
            .as_ref()
            .map(|status| status.settings.p2p.clone())
            .unwrap_or_default();
        let listening = status
            .as_ref()
            .is_some_and(|status| status.effective == RemoteAccessState::Listening);
        let pairing = status.as_ref().and_then(|status| status.pairing.clone());
        let pairing_active = self.ssh.p2p.code.is_some()
            || matches!(pairing, Some(PairingState::AwaitingConfirmation { .. }));
        if pairing_active && !self.ssh.p2p.polling {
            self.poll_pairing(cx);
        }
        let relay = self
            .ssh
            .p2p
            .relay
            .get_or_insert_with(|| {
                let initial = settings.relay_url.clone().unwrap_or_default();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("https://relay.example")
                        .default_value(initial)
                })
            })
            .clone();
        let state_key = match status.as_ref().map(|status| status.effective) {
            Some(RemoteAccessState::Listening) => UiTextKey::RemoteAccessListening,
            Some(RemoteAccessState::Starting) => UiTextKey::RemoteAccessStarting,
            Some(RemoteAccessState::Stopping) => UiTextKey::RemoteAccessStopping,
            Some(RemoteAccessState::Failed) => UiTextKey::RemoteAccessFailed,
            Some(RemoteAccessState::Disabled) => UiTextKey::RemoteAccessDisabled,
            None => UiTextKey::RemoteAccessUnavailable,
        };

        let header = div()
            .flex()
            .flex_col()
            .gap(style.spacing.sm)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(style.spacing.lg)
                    .child(div().text_base().child(text.get(UiTextKey::P2pTitle)))
                    .child(
                        yttt_button(
                            "p2p-toggle",
                            text.get(if listening {
                                UiTextKey::RemoteAccessDisable
                            } else {
                                UiTextKey::RemoteAccessEnable
                            }),
                            if listening {
                                YtttButtonVariant::Secondary
                            } else {
                                YtttButtonVariant::Primary
                            },
                            theme,
                            style,
                        )
                        .disabled(busy || pairing_active)
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.set_p2p(!listening, cx);
                        })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::P2pDescription)),
            );

        let status_row = div()
            .flex()
            .items_center()
            .gap(style.spacing.sm)
            .py(style.spacing.sm)
            .border_b(style.border.hairline)
            .border_color(theme.border_variant)
            .child(
                div()
                    .size(gpui::rems(0.375))
                    .rounded_full()
                    .bg(if listening {
                        theme.success
                    } else {
                        theme.text_subtle
                    }),
            )
            .child(div().text_sm().child(text.get(state_key)))
            .when_some(
                status.as_ref().and_then(|status| status.endpoint_id),
                |row, key| {
                    row.child(div().text_xs().text_color(theme.text_muted).child(format!(
                        "{} {}",
                        text.get(UiTextKey::P2pThisKey),
                        &key.to_string()[..8]
                    )))
                },
            );

        let relay_account = status
            .as_ref()
            .and_then(|status| status.relay_account.as_ref())
            .map(|account| {
                div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.xs)
                    .child(
                        div().text_sm().child(
                            text.get(UiTextKey::P2pRelayAccount)
                                .replace("{account}", &account.account_id.to_string())
                                .replace("{relay}", account.relay_url.as_deref().unwrap_or("—")),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::P2pRelayAccountDevices)),
                    )
                    .when_some(account.error.clone(), |view, error| {
                        view.child(div().text_xs().text_color(theme.danger).child(error))
                    })
            });

        let relay_row = div()
            .flex()
            .flex_col()
            .gap(style.spacing.sm)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::P2pRelayLabel)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(style.spacing.sm)
                    .child(
                        yttt_input(&relay, YtttInputKind::Settings, theme, style)
                            .disabled(busy || pairing_active)
                            .w(gpui::rems(18.0)),
                    )
                    .child(
                        yttt_button(
                            "p2p-relay-apply",
                            text.get(UiTextKey::RemoteAccessApply),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                        )
                        .disabled(busy || pairing_active || !listening)
                        .on_click(cx.listener(|root, _, _, cx| root.set_p2p(true, cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::P2pRelayHint)),
            );

        let pairing_view = self.pairing_view(pairing, listening, busy, cx);
        let devices = self.devices_view(busy, cx);

        div()
            .debug_selector(|| "p2p-settings".into())
            .flex()
            .flex_col()
            .gap(style.spacing.lg)
            .pt(style.spacing.lg)
            .border_t(style.border.hairline)
            .border_color(theme.border_variant)
            .child(header)
            .child(status_row)
            .children(relay_account)
            .child(relay_row)
            .child(pairing_view)
            .child(devices)
            .when_some(
                status.as_ref().and_then(|status| status.error.clone()),
                |view, error| view.child(div().text_sm().text_color(theme.danger).child(error)),
            )
    }

    fn pairing_view(
        &mut self,
        pairing: Option<PairingState>,
        listening: bool,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let view = div().flex().flex_col().gap(style.spacing.md);
        match pairing {
            Some(PairingState::AwaitingConfirmation {
                endpoint_id,
                device_label,
                short_code,
                ..
            }) => view
                .debug_selector(|| "p2p-confirm".into())
                .child(
                    div().text_sm().child(
                        text.get(UiTextKey::P2pAwaiting)
                            .replace("{device}", &device_label),
                    ),
                )
                .child(
                    div()
                        .debug_selector(|| "p2p-short-code".into())
                        .text_3xl()
                        .font_family("monospace")
                        .child(crate::ui::app::paired_host::format_short_code(short_code)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(style.spacing.sm)
                        .child(
                            yttt_button(
                                "p2p-allow-observe",
                                text.get(UiTextKey::P2pAllowObserve),
                                YtttButtonVariant::Secondary,
                                theme,
                                style,
                            )
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |root, _, _, cx| {
                                    root.begin_remote_access_action(
                                        Request::RemoteAccess(
                                            RemoteAccessRequest::ConfirmPairing {
                                                endpoint_id,
                                                preset: DevicePreset::Observe,
                                            },
                                        ),
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            yttt_button(
                                "p2p-allow-full",
                                text.get(UiTextKey::P2pAllowFull),
                                YtttButtonVariant::Secondary,
                                theme,
                                style,
                            )
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |root, _, _, cx| {
                                    root.begin_remote_access_action(
                                        Request::RemoteAccess(
                                            RemoteAccessRequest::ConfirmPairing {
                                                endpoint_id,
                                                preset: DevicePreset::Full,
                                            },
                                        ),
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            yttt_button(
                                "p2p-decline",
                                text.get(UiTextKey::P2pDecline),
                                YtttButtonVariant::Secondary,
                                theme,
                                style,
                            )
                            .disabled(busy)
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.begin_remote_access_action(
                                    Request::RemoteAccess(RemoteAccessRequest::CancelPairing),
                                    cx,
                                )
                            })),
                        ),
                ),
            Some(PairingState::Compromised) => view
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(text.get(UiTextKey::P2pCompromised)),
                )
                .child(
                    yttt_button(
                        "p2p-acknowledge",
                        text.get(UiTextKey::P2pAcknowledge),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.begin_remote_access_action(
                            Request::RemoteAccess(RemoteAccessRequest::CancelPairing),
                            cx,
                        )
                    })),
                ),
            Some(PairingState::Open { expires_millis }) if self.ssh.p2p.code.is_some() => {
                let seconds = expires_millis.saturating_sub(now_millis()) / 1000;
                let show = self.ssh.p2p.show_code;
                let code = self.ssh.p2p.code.as_ref().map(|code| code.0.clone());
                view.debug_selector(|| "p2p-invitation".into())
                    .child(
                        div().text_sm().child(
                            text.get(UiTextKey::P2pPairingOpen)
                                .replace("{seconds}", &seconds.to_string()),
                        ),
                    )
                    .children(self.ssh.p2p.qr.clone().map(qr_view))
                    .when(show, |view| {
                        view.child(
                            div()
                                .debug_selector(|| "p2p-code-text".into())
                                .text_xs()
                                .font_family("monospace")
                                .child(code.clone().unwrap_or_default()),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(style.spacing.sm)
                            .child(
                                yttt_button(
                                    "p2p-show-code",
                                    text.get(if show {
                                        UiTextKey::P2pHideCode
                                    } else {
                                        UiTextKey::P2pShowCode
                                    }),
                                    YtttButtonVariant::Ghost,
                                    theme,
                                    style,
                                )
                                .on_click(cx.listener(
                                    |root, _, _, cx| {
                                        root.ssh.p2p.show_code = !root.ssh.p2p.show_code;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                yttt_button(
                                    "p2p-copy-code",
                                    text.get(UiTextKey::P2pCopyCode),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                )
                                .on_click(cx.listener(
                                    |root, _, _, cx| {
                                        if let Some(code) = &root.ssh.p2p.code {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                code.0.clone(),
                                            ));
                                            root.ssh.p2p.copied = true;
                                            cx.notify();
                                        }
                                    },
                                )),
                            )
                            .child(
                                yttt_button(
                                    "p2p-cancel-pairing",
                                    text.get(UiTextKey::Cancel),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                )
                                .disabled(busy)
                                .on_click(cx.listener(
                                    |root, _, _, cx| {
                                        root.begin_remote_access_action(
                                            Request::RemoteAccess(
                                                RemoteAccessRequest::CancelPairing,
                                            ),
                                            cx,
                                        )
                                    },
                                )),
                            ),
                    )
                    .when(self.ssh.p2p.copied, |view| {
                        view.child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(text.get(UiTextKey::P2pCodeCopied)),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::P2pCodeWarning)),
                    )
            }
            _ => view.child(
                div().child(
                    yttt_button(
                        "p2p-pair-device",
                        text.get(UiTextKey::P2pPairDevice),
                        YtttButtonVariant::Primary,
                        theme,
                        style,
                    )
                    .disabled(busy || !listening)
                    .on_click(cx.listener(|root, _, _, cx| {
                        root.begin_remote_access_action(
                            Request::RemoteAccess(RemoteAccessRequest::StartPairing),
                            cx,
                        )
                    })),
                ),
            ),
        }
    }

    fn devices_view(&mut self, busy: bool, cx: &mut Context<Self>) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let devices = self.ssh.p2p.devices.clone();
        div()
            .flex()
            .flex_col()
            .gap(style.spacing.md)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(text.get(UiTextKey::P2pDevices)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(devices.len().to_string()),
                    ),
            )
            .when(devices.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::P2pNoDevices)),
                )
            })
            .children(devices.into_iter().map(|device| {
                let endpoint_id = device.endpoint_id;
                div()
                    .debug_selector(move || format!("p2p-device-{endpoint_id}"))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(style.spacing.lg)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap(style.spacing.xs)
                            .child(div().text_sm().truncate().child(device.label.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(device_summary(&device, text)),
                            ),
                    )
                    .child(
                        yttt_button(
                            SharedString::from(format!("p2p-revoke-{endpoint_id}")),
                            text.get(UiTextKey::P2pRevoke),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(
                            move |root, _, window, cx| root.confirm_revoke(endpoint_id, window, cx),
                        )),
                    )
            }))
    }

    fn set_p2p(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let relay_url = self
            .ssh
            .p2p
            .relay
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .filter(|url| !url.is_empty());
        if let Some(url) = &relay_url
            && !(url.starts_with("http://") || url.starts_with("https://"))
        {
            self.ssh.remote_access_error =
                Some(self.ui_text.get(UiTextKey::PairedHostRelayInvalid).into());
            cx.notify();
            return;
        }
        self.begin_remote_access_action(
            Request::RemoteAccess(RemoteAccessRequest::SetP2p { enabled, relay_url }),
            cx,
        );
    }

    fn confirm_revoke(
        &mut self,
        endpoint_id: EndpointKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let appearance = self.theme_runtime();
        let text = self.ui_text;
        let root = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let revoke = root.clone();
            let revoke_all = root.clone();
            alert
                .title(text.get(UiTextKey::P2pRevokeTitle))
                .description(text.get(UiTextKey::P2pRevokeDescription))
                .footer(
                    DialogFooter::new()
                        .child(
                            yttt_button(
                                "p2p-revoke-cancel",
                                text.get(UiTextKey::Cancel),
                                YtttButtonVariant::Secondary,
                                appearance.ui,
                                appearance.style,
                            )
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            yttt_button(
                                "p2p-revoke-reset-tls",
                                text.get(UiTextKey::P2pRevokeAndResetTls),
                                YtttButtonVariant::Secondary,
                                appearance.ui,
                                appearance.style,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = revoke_all.update(cx, |root, cx| {
                                    root.begin_remote_access_action(
                                        Request::RemoteAccess(RemoteAccessRequest::RevokeDevice {
                                            endpoint_id,
                                            reset_tls: true,
                                        }),
                                        cx,
                                    )
                                });
                                window.close_dialog(cx);
                            }),
                        )
                        .child(
                            yttt_button(
                                "p2p-revoke-confirm",
                                text.get(UiTextKey::P2pRevoke),
                                YtttButtonVariant::Primary,
                                appearance.ui,
                                appearance.style,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = revoke.update(cx, |root, cx| {
                                    root.begin_remote_access_action(
                                        Request::RemoteAccess(RemoteAccessRequest::RevokeDevice {
                                            endpoint_id,
                                            reset_tls: false,
                                        }),
                                        cx,
                                    )
                                });
                                window.close_dialog(cx);
                            }),
                        ),
                )
        });
    }

    /// Keeps the page current while an invitation is open, without taking the
    /// busy flag: a user action always wins over a refresh in flight.
    fn poll_pairing(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.local_runtime() else {
            return;
        };
        self.ssh.p2p.polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PAIRING_POLL).await;
                let runtime = runtime.clone();
                let status = cx
                    .background_executor()
                    .spawn(async move {
                        runtime.request_blocking(Request::RemoteAccess(RemoteAccessRequest::Status))
                    })
                    .await;
                let keep = this
                    .update(cx, |root, cx| root.pairing_polled(status, cx))
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn pairing_polled(
        &mut self,
        status: Result<Response, impl std::fmt::Display>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ssh.remote_access_busy {
            // A user action is in flight and brings its own status.
            return true;
        }
        if let Ok(Response::RemoteAccess(RemoteAccessResponse::Status(status))) = status {
            self.ssh.remote_access = Some(status);
        }
        let pairing = self
            .ssh
            .remote_access
            .as_ref()
            .and_then(|status| status.p2p.pairing.clone());
        let keep = match pairing {
            Some(PairingState::Open { .. }) => self.ssh.p2p.code.is_some(),
            Some(PairingState::AwaitingConfirmation { .. }) => true,
            Some(PairingState::Compromised) | None => false,
        };
        if !keep {
            self.ssh.p2p.polling = false;
            self.invitation_ended(cx);
            self.refresh_devices(cx);
        }
        cx.notify();
        keep
    }

    /// Forgets the invitation's code here, and on the clipboard if it is still
    /// what was copied.
    pub(super) fn invitation_ended(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.ssh.p2p.code.take() else {
            return;
        };
        self.ssh.p2p.qr = None;
        self.ssh.p2p.show_code = false;
        if std::mem::take(&mut self.ssh.p2p.copied)
            && cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|copied| copied.trim() == code.0)
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(String::new()));
        }
    }

    /// Withdraws an open invitation when its page goes away, so it never stays
    /// open with no one watching.
    pub(super) fn abandon_pairing(&mut self) {
        if self.ssh.p2p.code.take().is_none() {
            return;
        }
        self.ssh.p2p.qr = None;
        self.ssh.p2p.show_code = false;
        self.ssh.p2p.copied = false;
        if let Some(runtime) = self.local_runtime() {
            let _ = std::thread::Builder::new()
                .name("yttt-p2p-cancel".into())
                .spawn(move || {
                    let _ = runtime.request_blocking(Request::RemoteAccess(
                        RemoteAccessRequest::CancelPairing,
                    ));
                });
        }
    }

    fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.local_runtime() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let devices = cx
                .background_executor()
                .spawn(async move {
                    runtime
                        .request_blocking(Request::RemoteAccess(RemoteAccessRequest::ListDevices))
                })
                .await;
            let _ = this.update(cx, |root, cx| {
                if let Ok(Response::RemoteAccess(RemoteAccessResponse::Devices(devices))) = devices
                {
                    root.ssh.p2p.devices = devices;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Applies what a P2P request returned; called for every remote-access action.
    pub(super) fn p2p_action_result(
        &mut self,
        response: &RemoteAccessResponse,
        cx: &mut Context<Self>,
    ) {
        match response {
            RemoteAccessResponse::PairingInvitation { code, .. } => {
                self.ssh.p2p.qr = QrMatrix::encode(&code.0).map(Arc::new);
                self.ssh.p2p.code = Some(code.clone());
                self.ssh.p2p.show_code = false;
                self.ssh.p2p.copied = false;
                if !self.ssh.p2p.polling {
                    self.poll_pairing(cx);
                }
            }
            RemoteAccessResponse::Devices(devices) => self.ssh.p2p.devices = devices.clone(),
            RemoteAccessResponse::RelayNodes(nodes) => {
                self.ssh.relay_account.nodes = Some(nodes.clone());
            }
            RemoteAccessResponse::Status(status) => {
                if !matches!(
                    status.p2p.pairing,
                    Some(PairingState::Open { .. } | PairingState::AwaitingConfirmation { .. })
                ) {
                    self.invitation_ended(cx);
                }
                if status.p2p.effective == RemoteAccessState::Listening {
                    self.refresh_devices(cx);
                } else {
                    self.ssh.p2p.devices.clear();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub(super) fn tests_now_millis() -> u64 {
    now_millis()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn device_summary(device: &PairedDevice, text: UiText) -> String {
    let format = |millis: u64| {
        chrono::DateTime::from_timestamp_millis(millis as i64)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default()
    };
    let preset = text.get(match device.preset {
        DevicePreset::Full => UiTextKey::P2pPresetFull,
        DevicePreset::Observe => UiTextKey::P2pPresetObserve,
    });
    let mut summary = format!(
        "{preset} · {}",
        text.get(UiTextKey::P2pPairedAt)
            .replace("{time}", &format(device.paired_at_millis))
    );
    if let Some(seen) = device.last_seen_millis {
        summary.push_str(" · ");
        summary.push_str(
            &text
                .get(UiTextKey::P2pLastSeen)
                .replace("{time}", &format(seen)),
        );
    }
    summary
}

/// Paints the code's modules as squares in one pass, with the quiet zone the
/// format requires.
fn qr_view(matrix: Arc<QrMatrix>) -> impl IntoElement {
    const MODULE: f32 = 3.0;
    const QUIET: usize = 4;
    let side = px((matrix.width + 2 * QUIET) as f32 * MODULE);
    div()
        .debug_selector(|| "p2p-qr".into())
        .w(side)
        .h(side)
        .child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    window.paint_quad(gpui::fill(bounds, gpui::white()));
                    let module = px(MODULE);
                    for (index, dark) in matrix.dark.iter().enumerate() {
                        if !dark {
                            continue;
                        }
                        let x = (index % matrix.width + QUIET) as f32;
                        let y = (index / matrix.width + QUIET) as f32;
                        window.paint_quad(gpui::fill(
                            gpui::Bounds::new(
                                bounds.origin + gpui::point(module * x, module * y),
                                gpui::size(module, module),
                            ),
                            gpui::black(),
                        ));
                    }
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod qr_tests {
    use super::QrMatrix;

    #[test]
    fn a_pairing_code_survives_its_qr_code() {
        let code = yttt_protocol::p2p::PairingCode {
            host: yttt_protocol::EndpointKey([7; 32]),
            relay_url: Some("https://relay.example.com".into()),
            direct_addrs: vec![
                "192.168.1.20:51820".parse().unwrap(),
                "[2001:db8::20]:51820".parse().unwrap(),
            ],
            invitation_id: [1; 16],
            secret: [2; 32],
            environment_id: "7f6c1f0e-5d4b-4c3a-9b8a-1234567890ab".into(),
            profile_id: yttt_core::model::ids::ProfileId::new("default"),
        }
        .encode();
        let matrix = QrMatrix::encode(&code).unwrap();
        let quiet = 4;
        let side = matrix.width + 2 * quiet;
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(side * 4, side * 4, |x, y| {
            let (x, y) = (x / 4, y / 4);
            let inside = (quiet..quiet + matrix.width).contains(&x)
                && (quiet..quiet + matrix.width).contains(&y);
            if inside && matrix.dark[(y - quiet) * matrix.width + x - quiet] {
                0
            } else {
                255
            }
        });
        let grids = image.detect_grids();
        assert_eq!(grids.len(), 1);
        let (_, decoded) = grids[0].decode().unwrap();
        assert_eq!(decoded, code);
        // Large enough to need a real scanner, small enough for a screen.
        assert!(matrix.width <= 101, "QR width {}", matrix.width);
    }
}
