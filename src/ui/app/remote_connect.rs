use std::collections::VecDeque;

use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Context, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Root as ComponentRoot,
    button::{Button, ButtonVariants as _},
};

use crate::{
    remote_host::{RemoteConnectEvent, RemoteEnvironment},
    remote_launch::RemoteLaunch,
};

type ReadyCallback = Box<dyn FnOnce(RemoteEnvironment, &mut App)>;

pub(super) fn open(
    launch: RemoteLaunch,
    on_ready: impl FnOnce(RemoteEnvironment, &mut App) + 'static,
    cx: &mut App,
) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(600.0), px(440.0)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(520.0), px(360.0))),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|_| RemoteConnectView::new(launch, Box::new(on_ready)));
            view.update(cx, |view, cx| view.begin_connect(window, cx));

            let on_close = view.downgrade();
            window.on_window_should_close(cx, move |_window, cx| {
                let _ = on_close.update(cx, |view, _| view.cancel());
                true
            });

            cx.new(|cx| ComponentRoot::new(view, window, cx))
        },
    )?;
    Ok(())
}

struct RemoteConnectView {
    launch: RemoteLaunch,
    on_ready: Option<ReadyCallback>,
    status: String,
    error: Option<String>,
    prompts: VecDeque<ConnectPrompt>,
    connecting: bool,
    cancelled: bool,
}

enum ConnectPrompt {
    HostKey(yttt_ssh::HostKeyChallenge),
    Takeover {
        owner: String,
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
    fn new(launch: RemoteLaunch, on_ready: ReadyCallback) -> Self {
        Self {
            launch,
            on_ready: Some(on_ready),
            status: "Preparing remote connection…".to_string(),
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
        self.status = format!("Connecting to {}…", self.launch.target.label());
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
            RemoteConnectEvent::Status(status) => self.status = status,
            RemoteConnectEvent::HostKey(challenge) => {
                self.status = "SSH host key verification requires your approval.".to_string();
                self.prompts.push_back(ConnectPrompt::HostKey(challenge));
            }
            RemoteConnectEvent::Takeover {
                owner,
                force,
                answer,
            } => {
                self.status = "Remote workspace takeover requires your approval.".to_string();
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
                window.remove_window();
                on_ready(environment, cx);
            }
            Err(error) => {
                self.reject_prompts();
                self.status = "Remote connection failed.".to_string();
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
            "Continuing SSH connection…".to_string()
        } else {
            "SSH host key rejected.".to_string()
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
            "Taking over the remote workspace…".to_string()
        } else {
            "Remote workspace takeover cancelled.".to_string()
        };
        cx.notify();
    }

    fn host_label(&self) -> String {
        self.launch.target.label()
    }

    fn host_key_prompt(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(ConnectPrompt::HostKey(challenge)) = self.prompts.front() else {
            return div();
        };

        let mut prompt = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(16.0))
            .border(px(1.0))
            .border_color(cx.theme().warning)
            .rounded(px(8.0))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().warning_foreground)
                    .child("Verify SSH Host Key"),
            )
            .child(div().child(format!("Host: {}", challenge.host)))
            .child(div().child(format!("Port: {}", challenge.port)))
            .child(div().child(format!("Algorithm: {}", challenge.algorithm)))
            .child(div().child(format!("Fingerprint: {}", challenge.fingerprint)));
        if let Some(previous_fingerprint) = &challenge.previous_fingerprint {
            prompt = prompt.child(
                div()
                    .text_color(cx.theme().danger_foreground)
                    .child(format!(
                        "Changed key — previous fingerprint: {previous_fingerprint}"
                    )),
            );
        }
        prompt.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .mt(px(8.0))
                .child(
                    Button::new("remote-host-key-reject")
                        .danger()
                        .label("Reject")
                        .on_click(cx.listener(Self::reject_host_key)),
                )
                .child(
                    Button::new("remote-host-key-trust-once")
                        .secondary()
                        .label("Trust Once")
                        .on_click(cx.listener(Self::trust_host_key_once)),
                )
                .child(
                    Button::new("remote-host-key-trust-remember")
                        .primary()
                        .label("Trust and Remember")
                        .on_click(cx.listener(Self::trust_host_key_and_remember)),
                ),
        )
    }

    fn takeover_prompt(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(ConnectPrompt::Takeover { owner, force, .. }) = self.prompts.front() else {
            return div();
        };

        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(16.0))
            .border(px(1.0))
            .border_color(cx.theme().warning)
            .rounded(px(8.0))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().warning_foreground)
                    .child(if *force { "Previous Client Could Not Publish" } else { "Continue This Profile Here?" }),
            )
            .child(div().child(format!("Current owner: {owner}")))
            .child(div().child(if *force {
                "Force continuation restores only the last confirmed state. Unpublished edits on the previous Client may be missing."
            } else {
                "The previous Client will publish all workspaces automatically. All of its windows will become observers; existing terminal processes keep running."
            }))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .mt(px(8.0))
                    .child(
                        Button::new("remote-workspace-takeover-cancel")
                            .secondary()
                            .label(if *force { "Cancel Transfer" } else { "Observe Only" })
                            .on_click(cx.listener(Self::decline_takeover)),
                    )
                    .child(
                        Button::new("remote-workspace-takeover-confirm")
                            .danger()
                            .label(if *force { "Force Continue" } else { "Continue Here" })
                            .on_click(cx.listener(Self::approve_takeover)),
                    ),
            )
    }
}

impl Drop for RemoteConnectView {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Render for RemoteConnectView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let endpoint = self.host_label();
        let mut card = div()
            .w_full()
            .max_w(px(640.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .p(px(28.0))
            .border(px(1.0))
            .border_color(cx.theme().border)
            .rounded(px(12.0))
            .bg(cx.theme().secondary)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Connecting to Remote Workspace"),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(endpoint),
            )
            .child(div().child(self.status.clone()));

        if let Some(error) = &self.error {
            card = card
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(cx.theme().danger_foreground)
                        .child("Connection error"),
                )
                .child(
                    div()
                        .text_color(cx.theme().danger_foreground)
                        .child(error.clone()),
                );
        }

        match self.prompts.front() {
            Some(ConnectPrompt::HostKey(_)) => card = card.child(self.host_key_prompt(cx)),
            Some(ConnectPrompt::Takeover { .. }) => card = card.child(self.takeover_prompt(cx)),
            None => {}
        }

        let retry_available = self.error.is_some() && !self.connecting;
        card = card.child(
            div()
                .flex()
                .gap(px(8.0))
                .child(
                    Button::new("remote-connect-retry")
                        .primary()
                        .label("Retry")
                        .disabled(!retry_available)
                        .on_click(cx.listener(Self::retry)),
                )
                .child(
                    Button::new("remote-connect-cancel")
                        .secondary()
                        .label("Cancel Connection")
                        .on_click(cx.listener(Self::cancel_window)),
                ),
        );

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.0))
            .child(card)
    }
}
