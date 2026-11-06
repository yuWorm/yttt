//! The relay account part of "Remote access to this computer": sign in, see
//! the plan and usage, let this computer's Host use the account, sign out.
//!
//! The account only buys relay transport. Nothing here changes which devices
//! may reach this computer.

use super::*;
use yttt_control_client::{AccountView, ControlClient, ControlSession};
use yttt_protocol::{
    Request,
    remote_access::{RelayAccount, RemoteAccessRequest, RemoteAccessState, SecretText},
};

use crate::relay_account::{self, AccountDisplay};

const DEFAULT_CONTROL_URL: &str = "https://";

enum Outcome {
    SignedIn(AccountDisplay, Option<AccountView>),
    Refreshed(Option<AccountView>),
    SignedOut,
    Failed(String),
    UseForHost(Option<RelayAccount>),
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
        let p2p = self
            .ssh
            .remote_access
            .as_ref()
            .map(|status| status.p2p.clone())
            .unwrap_or_default();
        let host_uses_account = p2p.relay_account.is_some();
        let p2p_listening = p2p.effective == RemoteAccessState::Listening;

        let body =
            match display {
                None => div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.sm)
                    .child(field(
                        text.get(UiTextKey::RelayAccountControlUrl),
                        &control_url,
                        busy,
                        theme,
                        style,
                    ))
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
                    ),
                Some(display) => {
                    let mut view_lines = div().flex().flex_col().gap(style.spacing.xs);
                    if let Some(view) = &view {
                        view_lines = view_lines
                            .child(
                                div().text_xs().text_color(theme.text_muted).child(
                                    text.get(UiTextKey::RelayAccountPlan)
                                        .replace(
                                            "{plan}",
                                            if view.plan_name.is_empty() {
                                                &view.plan
                                            } else {
                                                &view.plan_name
                                            },
                                        )
                                        .replace("{relay}", &view.relay_url),
                                ),
                            )
                            .child(
                                div().text_xs().text_color(theme.text_muted).child(
                                    text.get(UiTextKey::RelayAccountUsage)
                                        .replace("{used}", &format_bytes(view.month_upstream_bytes))
                                        .replace("{quota}", &format_bytes(view.monthly_bytes))
                                        .replace("{devices}", &view.devices.to_string())
                                        .replace("{max}", &view.max_devices.to_string()),
                                ),
                            )
                            .when(view.suspended, |lines| {
                                lines.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.danger)
                                        .child(text.get(UiTextKey::RelayAccountSuspended)),
                                )
                            });
                    }
                    div()
                        .debug_selector(|| "relay-account-signed-in".into())
                        .flex()
                        .flex_col()
                        .gap(style.spacing.sm)
                        .child(
                            div().text_sm().child(
                                text.get(UiTextKey::RelayAccountSignedIn)
                                    .replace("{email}", &display.email),
                            ),
                        )
                        .child(view_lines)
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(style.spacing.sm)
                                .child(
                                    yttt_button(
                                        "relay-account-host",
                                        text.get(if host_uses_account {
                                            UiTextKey::RelayAccountStopForHost
                                        } else {
                                            UiTextKey::RelayAccountUseForHost
                                        }),
                                        YtttButtonVariant::Secondary,
                                        theme,
                                        style,
                                    )
                                    .disabled(busy || self.ssh.remote_access_busy || !p2p_listening)
                                    .on_click(cx.listener(
                                        move |root, _, _, cx| {
                                            root.use_relay_account_for_host(!host_uses_account, cx)
                                        },
                                    )),
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
                        .when(!p2p_listening, |view| {
                            view.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(text.get(UiTextKey::RelayAccountHostNeedsP2p)),
                            )
                        })
                }
            };

        div()
            .debug_selector(|| "relay-account".into())
            .flex()
            .flex_col()
            .gap(style.spacing.md)
            .pt(style.spacing.lg)
            .border_t(style.border.hairline)
            .border_color(theme.border_variant)
            .child(
                div()
                    .text_base()
                    .child(text.get(UiTextKey::RelayAccountTitle)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::RelayAccountDescription)),
            )
            .child(body)
            .when_some(error, |view, error| {
                view.child(div().text_sm().text_color(theme.danger).child(error))
            })
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
                let state = &mut root.ssh.relay_account;
                state.busy = false;
                match outcome {
                    Outcome::SignedIn(display, view) => {
                        state.display = Some(display);
                        state.view = view;
                    }
                    Outcome::Refreshed(view) => {
                        if view.is_none() {
                            state.error = Some(
                                root.ui_text
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
                    Outcome::UseForHost(account) => root.begin_remote_access_action(
                        Request::RemoteAccess(RemoteAccessRequest::SetRelayAccount { account }),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
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
