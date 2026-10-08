use std::{cell::Cell, collections::VecDeque, rc::Rc};

use crate::ui::{
    i18n::{UiText, UiTextKey},
    theme::{AppearanceState, current_ui_style, current_workbench_theme},
    workbench::shell::{bar::BarSections, titlebar::workbench_titlebar},
};
use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Context, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
    relative, size,
};
use gpui_component::{
    Icon, IconName, Root as ComponentRoot, Sizable as _, scroll::ScrollableElement as _,
    spinner::Spinner,
};
use yttt_ui::primitives::button::{YtttButtonVariant, yttt_button};

use crate::{
    remote_host::{RemoteConnectEvent, RemoteConnectStatus, RemoteControlOwner, RemoteEnvironment},
    remote_launch::RemoteLaunch,
};

type ReadyCallback = Box<dyn FnOnce(RemoteEnvironment, &mut App)>;

pub(super) fn open(
    launch: RemoteLaunch,
    on_ready: impl FnOnce(RemoteEnvironment, &mut App) + 'static,
    cx: &mut App,
) -> anyhow::Result<()> {
    let scale = cx
        .global::<AppearanceState>()
        .runtime()
        .typography
        .font_size
        / 16.0;
    let bounds = Bounds::centered(None, size(px(600.0 * scale), px(440.0 * scale)), cx);
    let mut options = super::workbench_window_options(bounds, launch.appearance.window.effect);
    options.window_min_size = Some(size(px(480.0), px(320.0)));
    cx.open_window(options, move |window, cx| {
        let view = cx.new(|cx| RemoteConnectView::new(launch, Box::new(on_ready), window, cx));
        view.update(cx, |view, cx| view.begin_connect(window, cx));

        let on_close = view.downgrade();
        window.on_window_should_close(cx, move |_window, cx| {
            let _ = on_close.update(cx, |view, cx| view.cancel(cx));
            true
        });

        cx.new(|cx| ComponentRoot::new(view, window, cx))
    })?;
    Ok(())
}

fn replacement_is_password(connection: &crate::config::ssh::SshConnectionConfig) -> bool {
    use crate::config::ssh::{CredentialKind, SshAuthPreference};
    connection.auth == SshAuthPreference::Password
        || (connection.auth == SshAuthPreference::Auto
            && connection
                .credential
                .as_ref()
                .map_or(connection.identity_file.is_none(), |credential| {
                    credential.kind == CredentialKind::LoginPassword
                }))
}

struct RemoteConnectView {
    launch: RemoteLaunch,
    on_ready: Option<ReadyCallback>,
    text: UiText,
    status: UiTextKey,
    status_detail: Option<String>,
    show_details: bool,
    scroll: ScrollHandle,
    focus: FocusHandle,
    error: Option<String>,
    prompts: VecDeque<ConnectPrompt>,
    connecting: bool,
    cancelled: bool,
    connected: Rc<Cell<bool>>,
    replacement_credentials: Option<gpui::Entity<gpui_component::input::InputState>>,
    remember_replacement: bool,
    event_sender: Option<flume::Sender<RemoteConnectEvent>>,
}

enum ConnectPrompt {
    HostKey(yttt_ssh::HostKeyChallenge),
    Takeover {
        owner: RemoteControlOwner,
        force: bool,
        answer: flume::Sender<bool>,
    },
}

impl ConnectPrompt {
    fn reject(self) {
        match self {
            Self::HostKey(challenge) => {
                let _ = challenge.respond(yttt_ssh::HostKeyDecision {
                    accept: false,
                    remember: false,
                });
            }
            Self::Takeover { answer, .. } => {
                let _ = answer.send(false);
            }
        }
    }
}

impl RemoteConnectView {
    fn new(
        launch: RemoteLaunch,
        on_ready: ReadyCallback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            text: UiText::new(launch.appearance.locale),
            launch,
            on_ready: Some(on_ready),
            status: UiTextKey::RemoteConnectPreparing,
            status_detail: None,
            show_details: false,
            scroll: ScrollHandle::new(),
            focus,
            error: None,
            prompts: VecDeque::new(),
            connecting: false,
            cancelled: false,
            connected: Rc::new(Cell::new(false)),
            replacement_credentials: None,
            remember_replacement: false,
            event_sender: None,
        }
    }

    fn begin_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting || self.cancelled || self.on_ready.is_none() {
            return;
        }
        let mut host_credential = None;
        if let Some(input) = &self.replacement_credentials {
            let secret = zeroize::Zeroizing::new(input.read(cx).value().to_string());
            if !secret.is_empty() {
                match &mut self.launch.target {
                    crate::remote_launch::RemoteTarget::ExistingHost {
                        connection_info, ..
                    } => match crate::remote_launch::ConnectionCode::decode(&secret) {
                        Ok(code)
                            if code.connection_info.environment_id
                                == connection_info.environment_id
                                && code.connection_info.profile_id
                                    == connection_info.profile_id =>
                        {
                            *connection_info = code.connection_info;
                            if self.remember_replacement
                                && let Some(crate::remote_launch::SavedRemoteTarget::Host {
                                    credential_id,
                                }) = &self.launch.saved_target
                            {
                                host_credential = Some((credential_id.clone(), secret.clone()));
                            }
                        }
                        _ => {
                            self.error = Some(self.text.get(UiTextKey::RemoteWrongHostCode).into());
                            cx.notify();
                            return;
                        }
                    },
                    crate::remote_launch::RemoteTarget::SshServer {
                        connection,
                        password,
                        passphrase,
                        save_password_as,
                    } => {
                        if replacement_is_password(connection) {
                            *password = Some(secret.to_string());
                            *save_password_as = self.remember_replacement.then(|| {
                                connection
                                    .credential
                                    .as_ref()
                                    .map(|credential| credential.id.clone())
                                    .unwrap_or_else(|| {
                                        yttt_core::model::ids::CredentialId::new(
                                            uuid::Uuid::new_v4().to_string(),
                                        )
                                    })
                            });
                        } else {
                            *passphrase = Some(secret.to_string());
                        }
                    }
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }

        self.connecting = true;
        self.error = None;
        self.status = UiTextKey::RemoteConnecting;
        self.status_detail = None;
        let (events, receiver) = flume::unbounded();
        self.event_sender = Some(events.clone());
        let launch = self.launch.clone();
        let connect = cx.background_executor().spawn(async move {
            if let Some((credential_id, secret)) = host_credential {
                super::existing_host::save_replacement_credential(
                    &launch.local_profile,
                    &credential_id,
                    &secret,
                )?;
            }
            crate::remote_host::connect(launch, events)
        });

        self.listen_for_events(receiver, cx);

        cx.spawn_in(window, async move |this, cx| {
            let result = connect.await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.finish_connect(result, window, cx)
            });
        })
        .detach();

        cx.notify();
    }

    fn listen_for_events(
        &self,
        receiver: flume::Receiver<RemoteConnectEvent>,
        cx: &mut Context<Self>,
    ) {
        let connected = self.connected.clone();
        let text = self.text;
        // The connector outlives this window. Reconnect challenges belong to the
        // Client application, not to the disposable initial-connect view.
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv_async().await {
                if !connected.get() {
                    if this
                        .update(cx, |view, cx| view.handle_event(event, cx))
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                match event {
                    RemoteConnectEvent::HostKey(challenge) => {
                        let detail = format!(
                            "{}\n{}:{}\n{}: {}\n{}: {}{}",
                            text.get(if challenge.previous_fingerprint.is_some() {
                                UiTextKey::SshHostKeyChangedDescription
                            } else {
                                UiTextKey::SshHostKeyDescription
                            }),
                            challenge.host,
                            challenge.port,
                            text.get(UiTextKey::RemoteKeyAlgorithm),
                            challenge.algorithm,
                            text.get(UiTextKey::SshHostKeyReceivedFingerprint),
                            challenge.fingerprint,
                            challenge
                                .previous_fingerprint
                                .as_ref()
                                .map(|previous| format!(
                                    "\n{}: {previous}",
                                    text.get(UiTextKey::SshHostKeySavedFingerprint)
                                ))
                                .unwrap_or_default(),
                        );
                        let answer = cx.update(|cx| {
                            cx.windows().into_iter().find_map(|handle| {
                                handle
                                    .update(cx, |_, window, cx| {
                                        window.prompt(
                                            gpui::PromptLevel::Warning,
                                            text.get(UiTextKey::SshHostKeyTitle),
                                            Some(&detail),
                                            &[
                                                text.get(UiTextKey::SshHostKeyReject),
                                                text.get(UiTextKey::SshHostKeyTrustOnce),
                                                text.get(
                                                    if challenge.previous_fingerprint.is_some() {
                                                        UiTextKey::SshHostKeyReplace
                                                    } else {
                                                        UiTextKey::SshHostKeyTrustAndSave
                                                    },
                                                ),
                                            ],
                                            cx,
                                        )
                                    })
                                    .ok()
                            })
                        });
                        let choice = match answer {
                            Some(answer) => answer.await.ok(),
                            None => None,
                        };
                        let _ = challenge.respond(yttt_ssh::HostKeyDecision {
                            accept: matches!(choice, Some(1 | 2)),
                            remember: choice == Some(2),
                        });
                    }
                    RemoteConnectEvent::Takeover { answer, .. } => {
                        let _ = answer.send(false);
                    }
                    RemoteConnectEvent::Status(_) => {}
                }
            }
        })
        .detach();
    }

    fn handle_event(&mut self, event: RemoteConnectEvent, cx: &mut Context<Self>) {
        if self.cancelled || !self.connecting {
            match event {
                RemoteConnectEvent::HostKey(challenge) => {
                    ConnectPrompt::HostKey(challenge).reject()
                }
                RemoteConnectEvent::Takeover {
                    owner,
                    force,
                    answer,
                } => {
                    ConnectPrompt::Takeover {
                        owner,
                        force,
                        answer,
                    }
                    .reject();
                }
                RemoteConnectEvent::Status(_) => {}
            }
            return;
        }

        match event {
            RemoteConnectEvent::Status(status) => {
                self.status_detail = None;
                self.status = match status {
                    RemoteConnectStatus::VerifyingHost => UiTextKey::RemoteConnectVerifyingHost,
                    RemoteConnectStatus::CheckingServer => UiTextKey::RemoteConnectCheckingServer,
                    RemoteConnectStatus::StartingHost => UiTextKey::RemoteConnectStartingHost,
                    RemoteConnectStatus::Ssh { state, error } => {
                        self.status_detail = error;
                        match state {
                            yttt_ssh::ConnectionState::Disconnected => {
                                UiTextKey::RemoteConnectSshDisconnected
                            }
                            yttt_ssh::ConnectionState::Connecting => {
                                UiTextKey::RemoteConnectSshConnecting
                            }
                            yttt_ssh::ConnectionState::VerifyingHostKey => {
                                UiTextKey::SshHostKeyTitle
                            }
                            yttt_ssh::ConnectionState::Authenticating => {
                                UiTextKey::RemoteConnectSshAuthenticating
                            }
                            yttt_ssh::ConnectionState::Connected => {
                                UiTextKey::RemoteConnectSshConnected
                            }
                            yttt_ssh::ConnectionState::Reconnecting => {
                                UiTextKey::RemoteConnectSshReconnecting
                            }
                            yttt_ssh::ConnectionState::Failed => UiTextKey::RemoteConnectFailed,
                        }
                    }
                };
            }
            RemoteConnectEvent::HostKey(challenge) => {
                self.status = UiTextKey::SshHostKeyTitle;
                self.prompts.push_back(ConnectPrompt::HostKey(challenge));
            }
            RemoteConnectEvent::Takeover {
                owner,
                force,
                answer,
            } => {
                self.status = UiTextKey::RemoteTakeoverTitle;
                self.prompts.push_back(ConnectPrompt::Takeover {
                    owner,
                    force,
                    answer,
                });
            }
        }
        cx.notify();
    }

    fn finish_connect(
        &mut self,
        result: Result<RemoteEnvironment, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connecting = false;
        if self.cancelled {
            return;
        }

        match result {
            Ok(environment) => {
                let Some(on_ready) = self.on_ready.take() else {
                    return;
                };
                self.connected.set(true);
                while let Some(prompt) = self.prompts.pop_front() {
                    match prompt {
                        ConnectPrompt::HostKey(challenge) => {
                            if let Some(events) = &self.event_sender {
                                let _ = events.send(RemoteConnectEvent::HostKey(challenge));
                            }
                        }
                        prompt => prompt.reject(),
                    }
                }
                // Keep a window alive until its replacements exist: remote Clients quit when
                // the last window closes.
                on_ready(environment, cx);
                window.remove_window();
            }
            Err(error) => {
                self.reject_prompts();
                self.status = UiTextKey::RemoteConnectFailed;
                self.error = Some(error);
                if self.launch.saved_target.is_some()
                    && self.replacement_credentials.is_none()
                    && !matches!(&self.launch.target, crate::remote_launch::RemoteTarget::SshServer { connection, .. }
                        if connection.auth == crate::config::ssh::SshAuthPreference::Agent)
                {
                    let placeholder =
                        self.text.get(match &self.launch.target {
                            crate::remote_launch::RemoteTarget::ExistingHost { .. } => {
                                UiTextKey::ConnectionCodePlaceholder
                            }
                            crate::remote_launch::RemoteTarget::SshServer {
                                connection, ..
                            } if replacement_is_password(connection) => UiTextKey::SshPassword,
                            _ => UiTextKey::SshKeyPassphrase,
                        });
                    self.replacement_credentials = Some(cx.new(|cx| {
                        gpui_component::input::InputState::new(window, cx)
                            .masked(true)
                            .placeholder(placeholder)
                    }));
                }
                cx.notify();
            }
        }
    }

    fn reject_prompts(&mut self) {
        while let Some(prompt) = self.prompts.pop_front() {
            prompt.reject();
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.cancelled {
            return;
        }
        self.cancelled = true;
        self.connecting = false;
        self.reject_prompts();
        if cx.has_global::<crate::remote_restore::RemoteRestoreGlobal>()
            && let Err(error) = cx
                .global::<crate::remote_restore::RemoteRestoreGlobal>()
                .cancel_connect()
        {
            eprintln!("cannot remove cancelled remote Client restore record: {error}");
        }
    }

    fn retry(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_connect(window, cx);
    }

    fn cancel_window(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(cx);
        window.remove_window();
    }

    fn reject_host_key(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_host_key(false, false, cx);
    }

    fn trust_host_key_once(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_host_key(true, false, cx);
    }

    fn trust_host_key_and_remember(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.answer_host_key(true, true, cx);
    }

    fn answer_host_key(&mut self, accept: bool, remember: bool, cx: &mut Context<Self>) {
        if !matches!(self.prompts.front(), Some(ConnectPrompt::HostKey(_))) {
            return;
        }
        let Some(ConnectPrompt::HostKey(challenge)) = self.prompts.pop_front() else {
            return;
        };
        let _ = challenge.respond(yttt_ssh::HostKeyDecision { accept, remember });
        self.status = if accept {
            UiTextKey::RemoteConnectContinuing
        } else {
            UiTextKey::RemoteConnectSshRejected
        };
        cx.notify();
    }

    fn decline_takeover(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_takeover(false, cx);
    }

    fn approve_takeover(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.answer_takeover(true, cx);
    }

    fn answer_takeover(&mut self, take_over: bool, cx: &mut Context<Self>) {
        if !matches!(self.prompts.front(), Some(ConnectPrompt::Takeover { .. })) {
            return;
        }
        let Some(ConnectPrompt::Takeover { answer, .. }) = self.prompts.pop_front() else {
            return;
        };
        let _ = answer.send(take_over);
        self.status = if take_over {
            UiTextKey::RemoteConnectTakingControl
        } else {
            UiTextKey::RemoteConnectObserving
        };
        cx.notify();
    }

    fn host_label(&self) -> String {
        self.launch.target.label()
    }

    fn detail(&self, label: UiTextKey, value: String, cx: &App) -> gpui::Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(style.spacing.xs)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(self.text.get(label)),
            )
            .child(div().text_sm().child(value))
    }

    fn prompt_body(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let text = self.text;
        let body = div().flex().flex_col().min_w_0().gap(style.spacing.lg);
        match self.prompts.front() {
            Some(ConnectPrompt::HostKey(challenge)) => {
                let changed = challenge.previous_fingerprint.is_some();
                body.child(
                    div()
                        .text_sm()
                        .text_color(if changed {
                            theme.danger
                        } else {
                            theme.text_muted
                        })
                        .child(text.get(if changed {
                            UiTextKey::SshHostKeyChangedDescription
                        } else {
                            UiTextKey::SshHostKeyDescription
                        })),
                )
                .child(self.detail(
                    UiTextKey::RemoteKeyAlgorithm,
                    challenge.algorithm.clone(),
                    cx,
                ))
                .child(self.detail(
                    UiTextKey::SshHostKeyReceivedFingerprint,
                    challenge.fingerprint.clone(),
                    cx,
                ))
                .children(challenge.previous_fingerprint.as_ref().map(|previous| {
                    self.detail(UiTextKey::SshHostKeySavedFingerprint, previous.clone(), cx)
                }))
            }
            Some(ConnectPrompt::Takeover { owner, force, .. }) => {
                let body = body
                    .child(
                        div()
                            .text_sm()
                            .text_color(if *force {
                                theme.danger
                            } else {
                                theme.text_muted
                            })
                            .child(text.get(if *force {
                                UiTextKey::RemoteTakeoverForceDescription
                            } else {
                                UiTextKey::RemoteTakeoverDescription
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(style.spacing.xl)
                            .child(
                                self.detail(UiTextKey::RemoteProfile, owner.profile_id.clone(), cx)
                                    .flex_1(),
                            )
                            .child(self.detail(
                                UiTextKey::RemoteWorkspaceCount,
                                owner.workspace_count.to_string(),
                                cx,
                            )),
                    )
                    .child(
                        div().flex().child(
                            yttt_button(
                                "remote-connect-details",
                                text.get(if self.show_details {
                                    UiTextKey::RemoteConnectHideDetails
                                } else {
                                    UiTextKey::RemoteConnectDetails
                                }),
                                YtttButtonVariant::Ghost,
                                theme,
                                style,
                                cx,
                            )
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_details = !view.show_details;
                                cx.notify();
                            })),
                        ),
                    );
                if self.show_details {
                    body.child(self.detail(
                        UiTextKey::RemoteEnvironment,
                        owner.environment_id.clone(),
                        cx,
                    ))
                    .child(
                        self.detail(
                            UiTextKey::RemoteClient,
                            owner
                                .client_id
                                .clone()
                                .unwrap_or_else(|| text.get(UiTextKey::RemoteUnowned).into()),
                            cx,
                        ),
                    )
                } else {
                    body
                }
            }
            None => body.children(
                self.error
                    .as_ref()
                    .or(self.status_detail.as_ref())
                    .map(|error| {
                        self.detail(UiTextKey::RemoteConnectErrorDetails, error.clone(), cx)
                    }),
            ).children(self.replacement_credentials.as_ref().map(|input| {
                div().flex().flex_col().gap_2()
                    .child(text.get(UiTextKey::RemoteCredentialsRequired))
                    .child(gpui_component::input::Input::new(input))
                    .when(matches!(&self.launch.target, crate::remote_launch::RemoteTarget::ExistingHost { .. })
                        || matches!(&self.launch.target, crate::remote_launch::RemoteTarget::SshServer { connection, .. }
                            if replacement_is_password(connection)), |body| {
                        body.child(yttt_button("remember-replacement", text.get(if self.remember_replacement {
                            UiTextKey::ConnectionRemembered
                        } else { UiTextKey::ConnectionRemember }), YtttButtonVariant::Secondary, theme, style, cx)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.remember_replacement = !view.remember_replacement;
                                cx.notify();
                            })))
                    })
                    .when(matches!(&self.launch.target, crate::remote_launch::RemoteTarget::SshServer { connection, .. }
                        if !replacement_is_password(connection)), |body| body.child(text.get(UiTextKey::RemotePassphraseTemporary)))
            })).when(self.error.is_some() && matches!(&self.launch.target,
                crate::remote_launch::RemoteTarget::SshServer { connection, .. }
                    if connection.auth == crate::config::ssh::SshAuthPreference::Agent),
                |body| body.child(text.get(UiTextKey::RemoteRepairSshAgent))),
        }
    }

    fn footer(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let text = self.text;
        let cancel = yttt_button(
            "remote-connect-cancel",
            text.get(UiTextKey::RemoteConnectCancel),
            YtttButtonVariant::Ghost,
            theme,
            style,
            cx,
        )
        .on_click(cx.listener(Self::cancel_window));
        let actions = div().flex().flex_wrap().justify_end().gap(style.spacing.sm);
        let actions = match self.prompts.front() {
            Some(ConnectPrompt::HostKey(challenge)) => actions
                .child(
                    yttt_button(
                        "remote-host-key-reject",
                        text.get(UiTextKey::SshHostKeyReject),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(Self::reject_host_key)),
                )
                .child(
                    yttt_button(
                        "remote-host-key-trust-once",
                        text.get(UiTextKey::SshHostKeyTrustOnce),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(Self::trust_host_key_once)),
                )
                .child(
                    yttt_button(
                        "remote-host-key-trust-remember",
                        text.get(if challenge.previous_fingerprint.is_some() {
                            UiTextKey::SshHostKeyReplace
                        } else {
                            UiTextKey::SshHostKeyTrustAndSave
                        }),
                        if challenge.previous_fingerprint.is_some() {
                            YtttButtonVariant::Danger
                        } else {
                            YtttButtonVariant::Primary
                        },
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(Self::trust_host_key_and_remember)),
                ),
            Some(ConnectPrompt::Takeover { force, .. }) => actions
                .child(
                    yttt_button(
                        "remote-workspace-takeover-cancel",
                        text.get(if *force {
                            UiTextKey::RemoteCancelTransfer
                        } else {
                            UiTextKey::RemoteObserveOnly
                        }),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(Self::decline_takeover)),
                )
                .child(
                    yttt_button(
                        "remote-workspace-takeover-confirm",
                        text.get(if *force {
                            UiTextKey::RemoteForceContinue
                        } else {
                            UiTextKey::RemoteContinueHere
                        }),
                        if *force {
                            YtttButtonVariant::Danger
                        } else {
                            YtttButtonVariant::Primary
                        },
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(Self::approve_takeover)),
                ),
            None => actions.when(self.error.is_some() && !self.connecting, |actions| {
                actions.child(
                    yttt_button(
                        "remote-connect-retry",
                        text.get(UiTextKey::Retry),
                        YtttButtonVariant::Primary,
                        theme,
                        style,
                        cx,
                    )
                    .on_click(cx.listener(Self::retry)),
                )
            }),
        };
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(style.spacing.sm)
            .p(style.spacing.lg)
            .border_t_1()
            .border_color(theme.border_variant)
            .child(cancel)
            .child(actions)
    }
}

impl Drop for RemoteConnectView {
    fn drop(&mut self) {
        self.reject_prompts();
    }
}

impl Render for RemoteConnectView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = cx.global::<AppearanceState>().runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        window.set_rem_size(px(appearance.typography.font_size));
        let title = self.text.get(UiTextKey::RemoteConnectTitle);
        window.set_window_title(&format!("{title} — yttt"));
        let heading = match self.prompts.front() {
            Some(ConnectPrompt::HostKey(challenge)) => {
                if challenge.previous_fingerprint.is_some() {
                    UiTextKey::SshHostKeyChangedTitle
                } else {
                    UiTextKey::SshHostKeyTitle
                }
            }
            Some(ConnectPrompt::Takeover { force, .. }) => {
                if *force {
                    UiTextKey::RemoteTakeoverForceTitle
                } else {
                    UiTextKey::RemoteTakeoverTitle
                }
            }
            None => self.status,
        };
        let pending = self.connecting && self.prompts.is_empty();
        let heading_row = div()
            .flex()
            .items_center()
            .gap(style.spacing.sm)
            .when(pending, |row| row.child(Spinner::new().small()))
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.text.get(heading)),
            );
        let body = self.prompt_body(cx);
        let footer = self.footer(cx);
        div()
            .debug_selector(|| "remote-connect-window".into())
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.app_background)
            .text_color(theme.text)
            .font_family(appearance.typography.font_family.clone())
            .line_height(relative(appearance.typography.line_height))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    view.cancel(cx);
                    window.remove_window();
                    cx.stop_propagation();
                }
            }))
            .child(workbench_titlebar(
                BarSections {
                    left: vec![div().child(title).into_any_element()],
                    ..Default::default()
                },
                theme,
                style,
                window,
            ))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(style.spacing.md)
                    .p(style.spacing.lg)
                    .border_b_1()
                    .border_color(theme.border_variant)
                    .child(
                        Icon::new(IconName::Globe)
                            .size(px(18.0))
                            .text_color(theme.icon_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .child(self.host_label()),
                    )
                    .child(div().text_xs().text_color(theme.text_muted).child(
                        match &self.launch.target {
                            crate::remote_launch::RemoteTarget::SshServer { .. } => "SSH",
                            crate::remote_launch::RemoteTarget::ExistingHost { .. } => "TLS",
                        },
                    )),
            )
            .child(
                div()
                    .id("remote-connect-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.scroll)
                    .p(style.spacing.lg)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .gap(style.spacing.lg)
                            .child(heading_row)
                            .child(body),
                    ),
            )
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use yttt_protocol::{
        Request, Response, ServerEvent,
        ssh::{CredentialAnswer, CredentialChallenge, CredentialChallengeKind, HostKeyDecision},
    };

    struct ChallengeHost {
        events: flume::Receiver<ServerEvent>,
        answers: flume::Sender<Request>,
    }

    impl yttt_ssh::HostTransportProxy for ChallengeHost {
        fn request(&self, request: Request) -> Result<Response, String> {
            self.answers.send(request).unwrap();
            Err("response unused by the one-way challenge answer".into())
        }
        fn events(&self) -> flume::Receiver<ServerEvent> {
            self.events.clone()
        }
    }

    #[gpui::test]
    fn reconnect_host_key_prompt_survives_the_initial_window(cx: &mut gpui::TestAppContext) {
        use crate::config::profile::{
            AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        let root = tempfile::tempdir().unwrap();
        let profile = AppProfile::scoped(
            yttt_core::model::ids::ProfileId::new("challenge-test"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            root.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        cx.skip_drawing();
        cx.background_executor.allow_parking();
        let text = UiText::english();
        let launch = cx.update(|cx| {
            gpui_component::init(cx);
            let (_, theme) = super::super::load_app_runtime(&profile.config_paths());
            cx.set_global(AppearanceState::new(theme));
            RemoteLaunch {
                appearance: crate::remote_launch::RemoteAppearance::capture(cx, text),
                local_profile: profile,
                saved_target: None,
                target: crate::remote_launch::RemoteTarget::SshServer {
                    connection: crate::config::ssh::SshConnectionConfig::new(
                        "test",
                        "example.test",
                        22,
                        "user",
                    ),
                    password: None,
                    passphrase: None,
                    save_password_as: None,
                },
            }
        });
        let (initial, initial_cx) = cx.add_window_view(|window, cx| {
            RemoteConnectView::new(launch.clone(), Box::new(|_, _| {}), window, cx)
        });
        let (events, receiver) = flume::unbounded();
        initial.update(initial_cx, |view, cx| {
            view.connected.set(true);
            view.listen_for_events(receiver, cx);
        });
        let weak = initial.downgrade();
        initial.update_in(initial_cx, |_, window, _| window.remove_window());
        drop(initial);
        cx.run_until_parked();
        assert!(weak.upgrade().is_none());
        let (_remaining, _) = cx.add_window_view(|window, cx| {
            RemoteConnectView::new(launch, Box::new(|_, _| {}), window, cx)
        });
        let (host_events, host_receiver) = flume::unbounded();
        let (answers, answer_receiver) = flume::unbounded();
        let service = yttt_ssh::TransportService::from_host(Arc::new(ChallengeHost {
            events: host_receiver,
            answers,
        }))
        .unwrap();
        let challenges = service.events();
        for (id, button, expected) in [
            (
                1,
                UiTextKey::SshHostKeyTrustOnce,
                HostKeyDecision::AcceptOnce,
            ),
            (2, UiTextKey::SshHostKeyReject, HostKeyDecision::Reject),
        ] {
            host_events
                .send(ServerEvent::CredentialChallenge(CredentialChallenge {
                    challenge_id: id,
                    connection_id: "test".into(),
                    attempt: 1,
                    kind: CredentialChallengeKind::HostKey {
                        host: "example.test".into(),
                        port: 22,
                        algorithm: "ssh-ed25519".into(),
                        fingerprint: "SHA256:new".into(),
                        previous_fingerprint: Some("SHA256:old".into()),
                    },
                }))
                .unwrap();
            let yttt_ssh::TransportEvent::HostKeyChallenge(challenge) =
                challenges.recv_blocking().unwrap()
            else {
                panic!("host-key challenge")
            };
            events.send(RemoteConnectEvent::HostKey(challenge)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cx.has_pending_prompt() {
                cx.run_until_parked();
                assert!(
                    Instant::now() < deadline,
                    "reconnect challenge lost with initial view"
                );
            }
            let (_, detail) = cx.pending_prompt().unwrap();
            assert!(detail.contains("SHA256:new") && detail.contains("SHA256:old"));
            cx.simulate_prompt_answer(text.get(button));
            cx.run_until_parked();
            let Request::CredentialAnswer {
                challenge_id,
                answer,
            } = answer_receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            else {
                panic!("credential answer")
            };
            assert_eq!(challenge_id, id);
            assert_eq!(answer, CredentialAnswer::HostKey(expected));
        }
    }
}
