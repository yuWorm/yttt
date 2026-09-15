use std::collections::VecDeque;

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
            let _ = on_close.update(cx, |view, _| view.cancel());
            true
        });

        cx.new(|cx| ComponentRoot::new(view, window, cx))
    })?;
    Ok(())
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
        }
    }

    fn begin_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting || self.cancelled || self.on_ready.is_none() {
            return;
        }

        self.connecting = true;
        self.error = None;
        self.status = UiTextKey::RemoteConnecting;
        self.status_detail = None;
        let (events, receiver) = flume::unbounded();
        let launch = self.launch.clone();
        let connect = cx
            .background_executor()
            .spawn(async move { crate::remote_host::connect(launch, events) });

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = receiver.recv_async().await {
                if this
                    .update_in(cx, |view, _window, cx| view.handle_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn_in(window, async move |this, cx| {
            let result = connect.await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.finish_connect(result, window, cx)
            });
        })
        .detach();

        cx.notify();
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
                self.reject_prompts();
                let Some(on_ready) = self.on_ready.take() else {
                    return;
                };
                // Keep a window alive until its replacements exist: remote Clients quit when
                // the last window closes.
                on_ready(environment, cx);
                window.remove_window();
            }
            Err(error) => {
                self.reject_prompts();
                self.status = UiTextKey::RemoteConnectFailed;
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn reject_prompts(&mut self) {
        while let Some(prompt) = self.prompts.pop_front() {
            prompt.reject();
        }
    }

    fn cancel(&mut self) {
        if self.cancelled {
            return;
        }
        self.cancelled = true;
        self.connecting = false;
        self.reject_prompts();
    }

    fn retry(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_connect(window, cx);
    }

    fn cancel_window(&mut self, _: &ClickEvent, window: &mut Window, _: &mut Context<Self>) {
        self.cancel();
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
            ),
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
        self.cancel();
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
                    view.cancel();
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
