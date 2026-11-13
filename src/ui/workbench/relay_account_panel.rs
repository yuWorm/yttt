//! The relay account part of "Remote access to this computer": sign in, see
//! the plan and usage, let this computer's Host use the account, sign out.
//!
//! The account only buys relay transport. Nothing here changes which devices
//! may reach this computer.

use super::*;
use yttt_control_client::{AccountView, ControlClient, ControlError, ControlSession};
use yttt_protocol::{
    Request,
    remote_access::{RelayAccount, RemoteAccessRequest, RemoteAccessState, SecretText},
};

use crate::relay_account::{self, AccountDisplay};

const DEFAULT_CONTROL_URL: &str = "https://";
/// How long a browser sign-in waits for the console's answer.
const BROWSER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

enum Outcome {
    SignedIn(AccountDisplay, Option<AccountView>),
    Refreshed(Option<AccountView>),
    SignedOut,
    Failed(String),
    UseForHost(Option<RelayAccount>),
    /// The browser sign-in was cancelled here.
    Cancelled,
}

impl WorkbenchView {
    pub(super) fn relay_account_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        if !self.ssh.relay_account.loaded {
            self.load_relay_account(cx);
        }
        let state = &mut self.ssh.relay_account;
        let busy = state.busy;
        let control_url = state
            .control_url
            .get_or_insert_with(|| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("https://control.example.com")
                        .default_value(DEFAULT_CONTROL_URL)
                })
            })
            .clone();
        let email = state
            .email
            .get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder("name@example.com"))
            })
            .clone();
        let password = state
            .password
            .get_or_insert_with(|| cx.new(|cx| InputState::new(window, cx).masked(true)))
            .clone();
        let display = state.display.clone();
        let view = state.view.clone();
        let error = state.error.clone();
        let password_form = state.password_form;
        let waiting = state.browser_cancel.is_some();
        let p2p = self
            .ssh
            .remote_access
            .as_ref()
            .map(|status| status.p2p.clone())
            .unwrap_or_default();
        let host_uses_account = p2p.relay_account.is_some();
        let p2p_listening = p2p.effective == RemoteAccessState::Listening;
        let node_section =
            (host_uses_account && p2p_listening).then(|| self.relay_node_section(&p2p, cx));

        let signed_out = display.is_none();
        let body = match display {
            None => {
                // Buttons keep their own width.
                let mut form = div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(style.spacing.sm)
                    .child(field(
                        text.get(UiTextKey::RelayAccountControlUrl),
                        &control_url,
                        busy || waiting,
                        theme,
                        style,
                    ));
                if password_form {
                    form = form
                        .child(field(
                            text.get(UiTextKey::RelayAccountEmail),
                            &email,
                            busy,
                            theme,
                            style,
                        ))
                        .child(field(
                            text.get(UiTextKey::RelayAccountPassword),
                            &password,
                            busy,
                            theme,
                            style,
                        ))
                        .child(
                            div().child(
                                yttt_button(
                                    "relay-account-sign-in",
                                    text.get(UiTextKey::RelayAccountSignIn),
                                    YtttButtonVariant::Primary,
                                    theme,
                                    style,
                                )
                                .disabled(busy)
                                .on_click(cx.listener(
                                    |root, _, window, cx| root.sign_in_relay_account(window, cx),
                                )),
                            ),
                        );
                } else if waiting {
                    form = form.child(
                        div()
                            .debug_selector(|| "relay-account-waiting".into())
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(style.spacing.sm)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.text_muted)
                                    .child(text.get(UiTextKey::RelayAccountWaitingForBrowser)),
                            )
                            .child(
                                yttt_button(
                                    "relay-account-cancel",
                                    text.get(UiTextKey::RelayAccountCancel),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                )
                                .on_click(cx.listener(
                                    |root, _, _, cx| {
                                        // Dropping the sender ends the wait.
                                        root.ssh.relay_account.browser_cancel = None;
                                        cx.notify();
                                    },
                                )),
                            ),
                    );
                } else {
                    form = form.child(
                        div().child(
                            yttt_button(
                                "relay-account-browser",
                                text.get(UiTextKey::RelayAccountSignInWithBrowser),
                                YtttButtonVariant::Primary,
                                theme,
                                style,
                            )
                            .disabled(busy)
                            .on_click(cx.listener(|root, _, _, cx| root.sign_in_with_browser(cx))),
                        ),
                    );
                }
                form.when(!waiting, |form| {
                    form.child(
                        div().child(
                            yttt_button(
                                "relay-account-switch",
                                text.get(if password_form {
                                    UiTextKey::RelayAccountUseBrowser
                                } else {
                                    UiTextKey::RelayAccountUsePassword
                                }),
                                YtttButtonVariant::Ghost,
                                theme,
                                style,
                            )
                            .disabled(busy)
                            .on_click(cx.listener(|root, _, _, cx| {
                                let state = &mut root.ssh.relay_account;
                                state.password_form = !state.password_form;
                                state.error = None;
                                cx.notify();
                            })),
                        ),
                    )
                })
            }
            Some(display) => {
                let plan = view.as_ref().map(|view| {
                    if view.plan_name.is_empty() {
                        view.plan.clone()
                    } else {
                        view.plan_name.clone()
                    }
                });
                let usage = view.as_ref().map(|view| {
                    text.get(UiTextKey::RelayAccountUsage)
                        .replace("{used}", &format_bytes(view.month_upstream_bytes))
                        .replace("{quota}", &format_bytes(view.monthly_bytes))
                        .replace("{devices}", &view.devices.to_string())
                        .replace("{max}", &view.max_devices.to_string())
                });
                let suspended = view.as_ref().is_some_and(|view| view.suspended);
                div()
                    .debug_selector(|| "relay-account-signed-in".into())
                    .flex()
                    .flex_col()
                    .gap(style.spacing.md)
                    .child(
                        div()
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
                                    .child(div().text_sm().truncate().child(match &plan {
                                        Some(plan) => format!("{} · {plan}", display.email),
                                        None => display.email.clone(),
                                    }))
                                    .children(usage.map(|usage| {
                                        div().text_xs().text_color(theme.text_muted).child(usage)
                                    })),
                            )
                            .child(
                                yttt_button(
                                    "relay-account-sign-out",
                                    text.get(UiTextKey::RelayAccountSignOut),
                                    YtttButtonVariant::Ghost,
                                    theme,
                                    style,
                                )
                                .disabled(busy)
                                .on_click(cx.listener(
                                    move |root, _, _, cx| {
                                        root.sign_out_relay_account(host_uses_account, cx)
                                    },
                                )),
                            ),
                    )
                    .when(suspended, |view| {
                        view.child(
                            div()
                                .text_sm()
                                .text_color(theme.danger)
                                .child(text.get(UiTextKey::RelayAccountSuspended)),
                        )
                    })
                    // Signed in elsewhere, or before signing in turned it on.
                    .when(!host_uses_account, |view| {
                        view.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .justify_between()
                                .gap(style.spacing.sm)
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.text_muted)
                                        .child(text.get(UiTextKey::RelayAccountNotUsedHere)),
                                )
                                .child(
                                    yttt_button(
                                        "relay-account-host",
                                        text.get(UiTextKey::RelayAccountUseForHost),
                                        YtttButtonVariant::Primary,
                                        theme,
                                        style,
                                    )
                                    .disabled(busy || self.ssh.remote_access_busy)
                                    .on_click(cx.listener(
                                        |root, _, _, cx| root.use_relay_account_for_host(true, cx),
                                    )),
                                ),
                        )
                    })
                    .children(node_section)
            }
        };

        super::remote_access::remote_card(theme, style)
            .debug_selector(|| "relay-account".into())
            .child(super::remote_access::remote_card_header(
                text.get(UiTextKey::RelayAccountTitle),
                None,
                None,
                theme,
                style,
            ))
            .when(signed_out, |card| {
                card.child(
                    div()
                        .text_sm()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::RelayAccountDescription)),
                )
            })
            .child(body)
            .when_some(error, |view, error| {
                view.child(div().text_sm().text_color(theme.danger).child(error))
            })
    }

    /// The node this computer's Host relays through, and the nodes it may
    /// move to with its latency to each.
    fn relay_node_section(
        &mut self,
        p2p: &yttt_protocol::remote_access::P2pStatus,
        cx: &mut Context<Self>,
    ) -> Div {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let text = self.ui_text;
        let busy = self.ssh.remote_access_busy;
        let state = &mut self.ssh.relay_account;
        if state.choosing_node && state.nodes.is_none() && !busy {
            // Asked once per opening; the reply fills the list.
            state.nodes = Some(Vec::new());
            self.begin_remote_access_action(
                Request::RemoteAccess(RemoteAccessRequest::ListRelayNodes),
                cx,
            );
        }
        let state = &self.ssh.relay_account;
        let needed = p2p
            .relay_account
            .as_ref()
            .and_then(|account| account.placement_needed.clone());
        // While the Host waits for a choice, every node, its old one included,
        // can be chosen.
        let current = p2p
            .relay_account
            .as_ref()
            .and_then(|account| account.node_id.clone())
            .filter(|_| needed.is_none())
            .unwrap_or_default();
        let nodes = state.nodes.clone().unwrap_or_default();
        let name_of = |id: &str| {
            nodes
                .iter()
                .find(|node| node.id == id)
                .map(|node| {
                    if node.region.is_empty() {
                        node.name.clone()
                    } else {
                        format!("{} · {}", node.name, node.region)
                    }
                })
                .unwrap_or_else(|| id.to_owned())
        };
        // A node to choose opens the list by itself.
        let choosing = state.choosing_node || needed.is_some();
        let current_name = if current.is_empty() {
            text.get(UiTextKey::RelayAccountNodeNone).to_owned()
        } else {
            name_of(&current)
        };
        let mut section = div()
            .debug_selector(|| "relay-account-node".into())
            .flex()
            .flex_col()
            .gap(style.spacing.sm)
            .pt(style.spacing.md)
            .border_t(style.border.hairline)
            .border_color(theme.border_variant)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(style.spacing.sm)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(style.spacing.md)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.text_muted)
                                    .child(text.get(UiTextKey::RelayAccountNodeLabel)),
                            )
                            .child(div().text_sm().child(current_name)),
                    )
                    .when(needed.is_none(), |row| {
                        row.child(
                            yttt_button(
                                "relay-account-choose-node",
                                text.get(if choosing {
                                    UiTextKey::RelayAccountNodeCollapse
                                } else {
                                    UiTextKey::RelayAccountChooseNode
                                }),
                                YtttButtonVariant::Ghost,
                                theme,
                                style,
                            )
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |root, _, _, cx| {
                                    root.ssh.relay_account.choosing_node = !choosing;
                                    root.ssh.relay_account.nodes = None;
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            );
        if let Some(needed) = &needed {
            use yttt_control_client::PlacementReason;
            let node = needed
                .current
                .as_ref()
                .map(|current| name_of(&current.placement.node_id))
                .unwrap_or_default();
            let key = match needed.reason {
                PlacementReason::Missing => UiTextKey::RelayAccountNodeMissing,
                PlacementReason::NodeUnavailable => UiTextKey::RelayAccountNodeUnavailable,
                PlacementReason::AddressChanged => UiTextKey::RelayAccountNodeMoved,
            };
            section = section.child(
                div()
                    .debug_selector(|| "relay-account-node-needed".into())
                    .px(style.spacing.md)
                    .py(style.spacing.sm)
                    .rounded(style.radius.control)
                    .bg(theme.warning.alpha(0.12))
                    .text_sm()
                    .text_color(theme.warning)
                    .child(text.get(key).replace("{node}", &node)),
            );
        }
        if !choosing {
            return section;
        }
        if state.nodes.is_none() && !busy {
            // The list opened by itself: ask for it once.
            self.ssh.relay_account.nodes = Some(Vec::new());
            self.begin_remote_access_action(
                Request::RemoteAccess(RemoteAccessRequest::ListRelayNodes),
                cx,
            );
        }
        section = section.child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(text.get(UiTextKey::RelayAccountNodeHint)),
        );
        if nodes.is_empty() {
            return section.child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::RelayAccountNodeLoading)),
            );
        }
        let count = nodes.len();
        let mut list = div()
            .flex()
            .flex_col()
            .rounded(style.radius.control)
            .border(style.border.hairline)
            .border_color(theme.border_variant);
        for (index, node) in nodes.iter().enumerate() {
            let latency = match node.latency_ms {
                Some(ms) => text
                    .get(UiTextKey::RelayAccountNodeLatency)
                    .replace("{ms}", &ms.to_string()),
                None => text.get(UiTextKey::RelayAccountNodeNoAnswer).to_owned(),
            };
            let load = (!node.full)
                .then(|| {
                    node.load_percent.map(|percent| {
                        text.get(UiTextKey::RelayAccountNodeLoad)
                            .replace("{percent}", &percent.to_string())
                    })
                })
                .flatten()
                .unwrap_or_default();
            let is_current = node.id == current;
            let node_id = node.id.clone();
            let trailing = if is_current {
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::RelayAccountNodeCurrent))
            } else if node.full {
                div()
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(text.get(UiTextKey::RelayAccountNodeFull))
            } else {
                div().child(
                    yttt_button(
                        gpui::SharedString::from(format!("relay-node-{}", node.id)),
                        text.get(UiTextKey::RelayAccountNodeUse),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                    )
                    .disabled(busy)
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.ssh.relay_account.choosing_node = false;
                        root.begin_remote_access_action(
                            Request::RemoteAccess(RemoteAccessRequest::SetRelayNode {
                                node_id: node_id.clone(),
                            }),
                            cx,
                        );
                    })),
                )
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(style.spacing.sm)
                    .px(style.spacing.md)
                    .py(style.spacing.sm)
                    .when(is_current, |row| row.bg(theme.element_selected))
                    .when(index + 1 < count, |row| {
                        row.border_b(style.border.hairline)
                            .border_color(theme.border_variant)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(style.spacing.xs)
                            .child(
                                div()
                                    .text_sm()
                                    .when(node.full && !is_current, |name| {
                                        name.text_color(theme.text_muted)
                                    })
                                    .child(name_of(&node.id)),
                            )
                            .child(
                                div().text_xs().text_color(theme.text_muted).child(
                                    [latency, load]
                                        .into_iter()
                                        .filter(|part| !part.is_empty())
                                        .collect::<Vec<_>>()
                                        .join(" · "),
                                ),
                            ),
                    )
                    .child(trailing),
            );
        }
        section.child(list)
    }

    fn load_relay_account(&mut self, cx: &mut Context<Self>) {
        self.ssh.relay_account.loaded = true;
        let Some(profile) = self.config_paths.profile().cloned() else {
            return;
        };
        match relay_account::read_display(&profile) {
            Ok(display) => self.ssh.relay_account.display = display,
            Err(error) => self.ssh.relay_account.error = Some(error),
        }
        if self.ssh.relay_account.display.is_some() {
            self.run_relay_account(cx, move || {
                let session = signed_in(&profile)?;
                Ok(Outcome::Refreshed(block_on(session.account()).ok()))
            });
        }
    }

    /// Opens the console in the browser and waits on a loopback port for
    /// the person to allow this app there.
    fn sign_in_with_browser(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.ssh.relay_account;
        if state.busy || state.browser_cancel.is_some() {
            return;
        }
        let control_url = state
            .control_url
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let Some(profile) = self.config_paths.profile().cloned() else {
            return;
        };
        let (cancel, cancelled) = flume::bounded::<()>(1);
        let (opened, open) = flume::bounded::<String>(1);
        state.browser_cancel = Some(cancel);
        state.error = None;
        let text = self.ui_text;
        // The wait can last minutes: its own thread, not the shared pool.
        let (done, finished) = flume::bounded::<Result<Outcome, String>>(1);
        let work = move || -> Result<Outcome, String> {
            let client = ControlClient::new(&control_url).map_err(|error| error.to_string())?;
            block_on(async move {
                let login = client
                    .begin_browser_login(&relay_account::machine_name())
                    .await
                    .map_err(|error| error.to_string())?;
                let _ = opened.send(login.authorize_url().to_owned());
                drop(opened);
                enum Waited {
                    Session(ControlSession),
                    Cancelled,
                    Failed(String),
                }
                let finished = async {
                    match login.finish().await {
                        Ok(session) => Waited::Session(session),
                        Err(ControlError::Denied) => {
                            Waited::Failed(text.get(UiTextKey::RelayAccountBrowserDenied).into())
                        }
                        Err(error) => Waited::Failed(error.to_string()),
                    }
                };
                let cancelled = async {
                    let _ = cancelled.recv_async().await;
                    Waited::Cancelled
                };
                let timed_out = async {
                    tokio::time::sleep(BROWSER_TIMEOUT).await;
                    Waited::Failed(text.get(UiTextKey::RelayAccountBrowserTimedOut).into())
                };
                let session = match futures_lite::future::or(
                    finished,
                    futures_lite::future::or(cancelled, timed_out),
                )
                .await
                {
                    Waited::Session(session) => session,
                    Waited::Cancelled => return Ok(Outcome::Cancelled),
                    Waited::Failed(error) => return Err(error),
                };
                // The app learns the email only now.
                let view = session.account().await.map_err(|error| error.to_string())?;
                let display = relay_account::save(
                    &profile,
                    &relay_account::session_store(&profile),
                    &view.email,
                    &session,
                )?;
                Ok(Outcome::SignedIn(display, Some(view)))
            })
        };
        std::thread::spawn(move || {
            let _ = done.send(work());
        });
        cx.spawn(async move |this, cx| {
            if let Ok(url) = open.recv_async().await {
                let _ = this.update(cx, |_, cx| cx.open_url(&url));
            }
            let outcome = finished
                .recv_async()
                .await
                .unwrap_or_else(|_| Err("the sign-in stopped".into()))
                .unwrap_or_else(Outcome::Failed);
            let _ = this.update(cx, |root, cx| {
                root.ssh.relay_account.browser_cancel = None;
                root.apply_relay_account_outcome(outcome, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn sign_in_relay_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.ssh.relay_account;
        let read = |input: &Option<Entity<InputState>>| {
            input
                .as_ref()
                .map(|input| input.read(cx).value().trim().to_string())
                .unwrap_or_default()
        };
        let control_url = read(&state.control_url);
        let email = read(&state.email);
        let password = zeroize::Zeroizing::new(read(&state.password));
        let Some(profile) = self.config_paths.profile().cloned() else {
            return;
        };
        if let Some(input) = &self.ssh.relay_account.password {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.run_relay_account(cx, move || {
            let client = ControlClient::new(&control_url).map_err(|error| error.to_string())?;
            let session = block_on(client.login(&email, &password)).map_err(|e| e.to_string())?;
            let display = relay_account::save(
                &profile,
                &relay_account::session_store(&profile),
                &email,
                &session,
            )?;
            Ok(Outcome::SignedIn(display, block_on(session.account()).ok()))
        });
    }

    fn sign_out_relay_account(&mut self, host_uses_account: bool, cx: &mut Context<Self>) {
        let Some(profile) = self.config_paths.profile().cloned() else {
            return;
        };
        if host_uses_account {
            self.begin_remote_access_action(
                Request::RemoteAccess(RemoteAccessRequest::SetRelayAccount { account: None }),
                cx,
            );
        }
        self.run_relay_account(cx, move || {
            relay_account::forget(&profile, &relay_account::session_store(&profile))?;
            Ok(Outcome::SignedOut)
        });
    }

    fn use_relay_account_for_host(&mut self, enable: bool, cx: &mut Context<Self>) {
        if !enable {
            self.begin_remote_access_action(
                Request::RemoteAccess(RemoteAccessRequest::SetRelayAccount { account: None }),
                cx,
            );
            return;
        }
        let Some(profile) = self.config_paths.profile().cloned() else {
            return;
        };
        // Read from the credential store off the UI thread; it may prompt.
        self.run_relay_account(cx, move || {
            let session = signed_in(&profile)?;
            Ok(Outcome::UseForHost(Some(RelayAccount {
                control_url: session.control_url.clone(),
                account_id: session.account_id,
                session: SecretText(session.token.clone()),
            })))
        });
    }

    fn run_relay_account(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<Outcome, String> + Send + 'static,
    ) {
        if self.ssh.relay_account.busy {
            return;
        }
        self.ssh.relay_account.busy = true;
        self.ssh.relay_account.error = None;
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn(async move |this, cx| {
            let outcome = task.await.unwrap_or_else(Outcome::Failed);
            let _ = this.update(cx, |root, cx| {
                root.ssh.relay_account.busy = false;
                root.apply_relay_account_outcome(outcome, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_relay_account_outcome(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        let state = &mut self.ssh.relay_account;
        match outcome {
            Outcome::SignedIn(display, view) => {
                state.display = Some(display);
                state.view = view;
                // Signing in here is for this computer: use the account and
                // open remote access.
                self.use_relay_account_for_host(true, cx);
            }
            Outcome::Refreshed(view) => {
                if view.is_none() {
                    state.error = Some(
                        self.ui_text
                            .get(UiTextKey::RelayAccountSessionExpired)
                            .into(),
                    );
                }
                state.view = view;
            }
            Outcome::SignedOut => {
                state.display = None;
                state.view = None;
            }
            Outcome::Failed(error) => state.error = Some(error),
            Outcome::Cancelled => {}
            Outcome::UseForHost(account) => {
                state.nodes = None;
                let open =
                    account.is_some()
                        && self.ssh.remote_access.as_ref().is_none_or(|status| {
                            status.p2p.effective != RemoteAccessState::Listening
                        });
                let mut requests = vec![Request::RemoteAccess(
                    RemoteAccessRequest::SetRelayAccount { account },
                )];
                if open {
                    requests.push(Request::RemoteAccess(RemoteAccessRequest::SetP2p {
                        enabled: true,
                    }));
                }
                self.begin_remote_access_actions(requests, cx);
            }
        }
        cx.notify();
    }
}

fn signed_in(profile: &crate::config::profile::AppProfile) -> Result<ControlSession, String> {
    relay_account::load_session(&relay_account::session_store(profile))?
        .ok_or_else(|| "The relay account session is missing; sign in again.".into())
}

/// Runs a control-plane call to completion from a background thread.
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime")
        .block_on(future)
}

fn field(
    label: &'static str,
    input: &Entity<InputState>,
    busy: bool,
    theme: crate::ui::theme::WorkbenchTheme,
    style: yttt_ui::style::UiStyle,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(style.spacing.xs)
        .child(div().text_xs().text_color(theme.text_muted).child(label))
        .child(
            yttt_input(input, YtttInputKind::Settings, theme, style)
                .disabled(busy)
                .w(gpui::rems(20.0)),
        )
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn byte_counts_read_in_binary_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(50 * 1024 * 1024 * 1024), "50.0 GB");
    }
}
