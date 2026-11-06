//! Pairing this Client with a Host by code, and the Hosts it paired with.
//!
//! Pairing binds a fresh device key, kept only once the Host user accepts it.
//! The numbers shown here must match the Host's screen; that comparison is
//! what keeps someone else holding the code from pairing in this device's place.

use std::{net::SocketAddr, sync::Arc};

use gpui::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, StatefulInteractiveElement as _, Styled as _, Window,
    div, px,
};
use gpui_base::input::{InputEvent, InputState};
use yttt_protocol::{
    EndpointKey,
    p2p::{MAX_DEVICE_LABEL_BYTES, MAX_RELAY_URL_BYTES, PairingCode},
};
use yttt_ui::{
    primitives::{dialog::DialogFooter, scroll::ScrollableElement as _},
    window::WindowExt as _,
};

use crate::{
    config::profile::AppProfile,
    paired_hosts::{self, PairedHostRecord},
    remote_launch::{RemoteAppearance, RemoteLaunch, RemoteTarget},
    remote_restore::SavedRemoteTarget,
    ui::{
        i18n::{UiText, UiTextKey},
        primitives::{
            button::{YtttButtonVariant, yttt_button},
            input::{YtttInputKind, yttt_input},
        },
        theme::{current_ui_style, current_workbench_theme},
    },
};

/// Longest pairing code accepted from the input, well above any real one.
const MAX_CODE_INPUT_BYTES: usize = 2048;

#[derive(Clone)]
enum Editor {
    Closed,
    Pair(PairStage),
    Edit { host: EndpointKey },
}

#[derive(Clone)]
enum PairStage {
    Input,
    Reaching,
    Compare { short_code: u32 },
}

enum PairEvent {
    ShortCode(u32),
    Done(Result<PairedHostRecord, String>),
}

pub(crate) struct PairedHostForm {
    profile: AppProfile,
    text: UiText,
    name: Entity<InputState>,
    code: Entity<InputState>,
    relay: Entity<InputState>,
    addresses: Entity<InputState>,
    records: Vec<PairedHostRecord>,
    editor: Editor,
    busy: bool,
    error: Option<String>,
    generation: u64,
    cancel_pairing: Option<flume::Sender<()>>,
    scroll: ScrollHandle,
    _subscription: gpui::Subscription,
}

pub(crate) fn create(
    profile: AppProfile,
    text: UiText,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<PairedHostForm> {
    cx.new(|cx| {
        let code = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(text.get(UiTextKey::PairingCodePlaceholder))
                .validate(|value, _| value.len() <= MAX_CODE_INPUT_BYTES)
        });
        let subscription = cx.subscribe_in(
            &code,
            window,
            |view: &mut PairedHostForm, _, event, _, cx| {
                if matches!(event, InputEvent::Change) && view.error.is_some() {
                    view.error = None;
                    cx.notify();
                }
            },
        );
        let records = paired_hosts::read_records(&profile);
        let (records, error) = match records {
            Ok(records) => (records, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        PairedHostForm {
            profile,
            text,
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(text.get(UiTextKey::RemoteConnectionName))
                    .validate(|value, _| value.len() <= 1024)
            }),
            code,
            relay: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("https://relay.example")
                    .validate(|value, _| value.len() <= MAX_RELAY_URL_BYTES)
            }),
            addresses: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("192.168.1.20:51820")
                    .validate(|value, _| value.len() <= 1024)
            }),
            records,
            editor: Editor::Closed,
            busy: false,
            error,
            generation: 0,
            cancel_pairing: None,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    })
}

impl PairedHostForm {
    pub(crate) fn records(&self) -> &[PairedHostRecord] {
        &self.records
    }

    pub(crate) fn busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn editor_open(&self) -> bool {
        !matches!(self.editor, Editor::Closed)
    }

    pub(crate) fn title(&self) -> UiTextKey {
        match self.editor {
            Editor::Edit { .. } => UiTextKey::PairedHostEditTitle,
            _ => UiTextKey::PairedHostTitle,
        }
    }

    pub(crate) fn new_pairing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.clear_inputs(window, cx);
        self.error = None;
        self.editor = Editor::Pair(PairStage::Input);
        self.code.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn edit(&mut self, host: EndpointKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(record) = paired_hosts::find(&self.records, host).cloned() else {
            return;
        };
        let addresses = record
            .host
            .direct_addrs
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        self.name
            .update(cx, |input, cx| input.set_value(&record.name, window, cx));
        self.relay.update(cx, |input, cx| {
            input.set_value(record.host.relay_url.as_deref().unwrap_or(""), window, cx)
        });
        self.addresses
            .update(cx, |input, cx| input.set_value(&addresses, window, cx));
        self.error = None;
        self.editor = Editor::Edit { host };
        self.name.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn dismiss_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cancel) = self.cancel_pairing.take() {
            let _ = cancel.send(());
        }
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.editor = Editor::Closed;
        self.clear_inputs(window, cx);
        cx.notify();
    }

    fn clear_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.name, &self.code, &self.relay, &self.addresses] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn start_pairing(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let code_text = zeroize::Zeroizing::new(self.code.read(cx).value().to_string());
        let code = match PairingCode::decode(code_text.trim()) {
            Ok(code) => code,
            Err(_) => {
                self.error = Some(self.text.get(UiTextKey::PairingCodeInvalid).into());
                cx.notify();
                return;
            }
        };
        let name = self.name.read(cx).value().trim().to_string();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.busy = true;
        self.error = None;
        self.editor = Editor::Pair(PairStage::Reaching);
        let (events, receiver) = flume::unbounded();
        let (cancel, cancelled) = flume::bounded(1);
        self.cancel_pairing = Some(cancel);
        let profile = self.profile.clone();
        let spawned = std::thread::Builder::new()
            .name("yttt-p2p-pairing".into())
            .spawn(move || {
                let result = pair_with_host(&profile, code, name, &events, cancelled);
                let _ = events.send(PairEvent::Done(result));
            });
        if let Err(error) = spawned {
            self.busy = false;
            self.editor = Editor::Pair(PairStage::Input);
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = receiver.recv_async().await {
                let finished = matches!(event, PairEvent::Done(_));
                let current = this.update_in(cx, |view, window, cx| {
                    if view.generation != generation {
                        return false;
                    }
                    match event {
                        PairEvent::ShortCode(short_code) => {
                            view.editor = Editor::Pair(PairStage::Compare { short_code });
                        }
                        PairEvent::Done(result) => {
                            view.busy = false;
                            view.cancel_pairing = None;
                            match result {
                                Ok(record) => {
                                    let host = record.host.host;
                                    let _ = paired_hosts::upsert(&mut view.records, record);
                                    view.dismiss_editor(window, cx);
                                    if connect {
                                        view.connect_saved(host, window, cx);
                                    }
                                }
                                Err(error) => {
                                    view.editor = Editor::Pair(PairStage::Input);
                                    view.error = Some(error);
                                }
                            }
                        }
                    }
                    cx.notify();
                    true
                });
                if finished || !matches!(current, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn save_edit(&mut self, host: EndpointKey, window: &mut Window, cx: &mut Context<Self>) {
        let relay = self.relay.read(cx).value().trim().to_string();
        if !relay.is_empty() && !(relay.starts_with("http://") || relay.starts_with("https://")) {
            self.error = Some(self.text.get(UiTextKey::PairedHostRelayInvalid).into());
            cx.notify();
            return;
        }
        let addresses = self.addresses.read(cx).value().to_string();
        let mut direct_addrs = Vec::new();
        for address in addresses
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            match address.parse::<SocketAddr>() {
                Ok(address) => direct_addrs.push(address),
                Err(_) => {
                    self.error = Some(self.text.get(UiTextKey::PairedHostAddressesInvalid).into());
                    cx.notify();
                    return;
                }
            }
        }
        direct_addrs.truncate(yttt_protocol::p2p::MAX_DIRECT_ADDRS);
        let mut records = self.records.clone();
        let Some(record) = records.iter_mut().find(|record| record.host.host == host) else {
            self.dismiss_editor(window, cx);
            return;
        };
        record.name = self.name.read(cx).value().trim().to_string();
        record.host.relay_url = (!relay.is_empty()).then_some(relay);
        record.host.direct_addrs = direct_addrs;
        match paired_hosts::save_records(&self.profile, &records) {
            Ok(()) => {
                self.records = records;
                self.dismiss_editor(window, cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn connect_saved(
        &mut self,
        host: EndpointKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(record) = paired_hosts::find(&self.records, host).cloned() else {
            self.error = Some(self.text.get(UiTextKey::RemoteRecordMissing).into());
            cx.notify();
            return;
        };
        self.busy = true;
        self.error = None;
        let launch = RemoteLaunch {
            local_profile: self.profile.clone(),
            appearance: RemoteAppearance::capture(cx, self.text),
            saved_target: Some(SavedRemoteTarget::Paired { host }),
            target: RemoteTarget::PairedHost {
                name: record.name,
                host: record.host,
            },
        };
        let task = cx.background_spawn(async move {
            crate::remote_launch::spawn_remote_client(launch).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                view.error = result.err();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks before forgetting: the key cannot come back without pairing again.
    pub(crate) fn forget(
        &mut self,
        host: EndpointKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || paired_hosts::find(&self.records, host).is_none() {
            return;
        }
        let text = self.text;
        let this = cx.weak_entity();
        let appearance = cx.global::<crate::ui::theme::AppearanceState>().runtime();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let this = this.clone();
            alert
                .title(text.get(UiTextKey::PairedHostForgetTitle))
                .description(text.get(UiTextKey::PairedHostForgetDescription))
                .footer(
                    DialogFooter::new()
                        .child(
                            yttt_button(
                                "paired-host-forget-cancel",
                                text.get(UiTextKey::Cancel),
                                YtttButtonVariant::Secondary,
                                appearance.ui,
                                appearance.style,
                            )
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            yttt_button(
                                "paired-host-forget-confirm",
                                text.get(UiTextKey::PairedHostForget),
                                YtttButtonVariant::Primary,
                                appearance.ui,
                                appearance.style,
                            )
                            .on_click(move |_, window, cx| {
                                let _ = this.update(cx, |view, cx| view.forget_now(host, cx));
                                window.close_dialog(cx);
                            }),
                        ),
                )
        });
    }

    fn forget_now(&mut self, host: EndpointKey, cx: &mut Context<Self>) {
        let mut records = self.records.clone();
        records.retain(|record| record.host.host != host);
        let profile = self.profile.clone();
        remove_from_account(&profile, host);
        let result = paired_hosts::save_records(&profile, &records)
            .and_then(|()| {
                paired_hosts::delete_device_key(&paired_hosts::device_key_store(&profile), host)
            })
            .and_then(|()| {
                crate::remote_restore::forget(&profile, &SavedRemoteTarget::Paired { host })
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => self.records = records,
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn render_pair(&mut self, stage: PairStage, cx: &mut Context<Self>) -> Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let text = self.text;
        let body = match stage {
            PairStage::Input => div()
                .flex()
                .flex_col()
                .gap(style.spacing.md)
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::PairedHostDescription)),
                )
                .child(
                    div()
                        .text_sm()
                        .child(text.get(UiTextKey::RemoteConnectionName)),
                )
                .child(yttt_input(
                    &self.name,
                    YtttInputKind::Settings,
                    theme,
                    style,
                ))
                .child(div().text_sm().child(text.get(UiTextKey::PairingCodeLabel)))
                .child(yttt_input(
                    &self.code,
                    YtttInputKind::Settings,
                    theme,
                    style,
                )),
            PairStage::Reaching => div()
                .text_sm()
                .text_color(theme.text_muted)
                .child(text.get(UiTextKey::PairingReaching)),
            PairStage::Compare { short_code } => div()
                .flex()
                .flex_col()
                .gap(style.spacing.md)
                .child(div().text_sm().child(text.get(UiTextKey::PairingCompare)))
                .child(
                    div()
                        .debug_selector(|| "pairing-short-code".into())
                        .text_3xl()
                        .font_family("monospace")
                        .child(format_short_code(short_code)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::PairingCompareHint)),
                ),
        };
        let pairing = !matches!(self.editor, Editor::Pair(PairStage::Input));
        self.frame(
            body,
            div()
                .flex()
                .gap(style.spacing.sm)
                .child(
                    yttt_button(
                        "paired-host-cancel",
                        text.get(UiTextKey::Cancel),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                    )
                    .on_click(cx.listener(|view, _, window, cx| view.dismiss_editor(window, cx))),
                )
                .child(
                    yttt_button(
                        "paired-host-pair",
                        text.get(UiTextKey::PairingStart),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                    )
                    .disabled(pairing)
                    .on_click(
                        cx.listener(|view, _, window, cx| view.start_pairing(false, window, cx)),
                    ),
                )
                .child(
                    yttt_button(
                        "paired-host-pair-connect",
                        text.get(UiTextKey::RemoteSaveConnect),
                        YtttButtonVariant::Primary,
                        theme,
                        style,
                    )
                    .disabled(pairing)
                    .on_click(
                        cx.listener(|view, _, window, cx| view.start_pairing(true, window, cx)),
                    ),
                ),
            cx,
        )
    }

    fn render_edit(&mut self, host: EndpointKey, cx: &mut Context<Self>) -> Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let text = self.text;
        let body = div()
            .flex()
            .flex_col()
            .gap(style.spacing.md)
            .child(
                div()
                    .text_sm()
                    .child(text.get(UiTextKey::RemoteConnectionName)),
            )
            .child(yttt_input(
                &self.name,
                YtttInputKind::Settings,
                theme,
                style,
            ))
            .child(
                div()
                    .text_sm()
                    .child(text.get(UiTextKey::PairedHostRelayLabel)),
            )
            .child(yttt_input(
                &self.relay,
                YtttInputKind::Settings,
                theme,
                style,
            ))
            .child(
                div()
                    .text_sm()
                    .child(text.get(UiTextKey::PairedHostAddressesLabel)),
            )
            .child(yttt_input(
                &self.addresses,
                YtttInputKind::Settings,
                theme,
                style,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::PairedHostAddressesHint)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(text.get(UiTextKey::PairedHostRepair)),
            );
        self.frame(
            body,
            div()
                .flex()
                .gap(style.spacing.sm)
                .child(
                    yttt_button(
                        "paired-host-edit-cancel",
                        text.get(UiTextKey::Cancel),
                        YtttButtonVariant::Secondary,
                        theme,
                        style,
                    )
                    .on_click(cx.listener(|view, _, window, cx| view.dismiss_editor(window, cx))),
                )
                .child(
                    yttt_button(
                        "paired-host-edit-save",
                        text.get(UiTextKey::SettingsSave),
                        YtttButtonVariant::Primary,
                        theme,
                        style,
                    )
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.save_edit(host, window, cx)),
                    ),
                ),
            cx,
        )
    }

    fn frame(&self, body: Div, actions: Div, cx: &mut Context<Self>) -> Div {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        div()
            .debug_selector(|| "paired-host-editor".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(theme.text)
            .child(
                div()
                    .id("paired-host-form-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(style.spacing.md)
                    .p(px(20.0))
                    .child(body)
                    .children(self.error.clone().map(|error| div().text_sm().child(error))),
            )
            .child(
                div()
                    .id("paired-host-form-footer")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .p(px(12.0))
                    .border_t_1()
                    .border_color(theme.border_variant)
                    .child(actions),
            )
    }
}

impl Render for PairedHostForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.editor.clone() {
            Editor::Closed => div().debug_selector(|| "paired-host-controller".into()),
            Editor::Pair(stage) => self.render_pair(stage, cx),
            Editor::Edit { host } => self.render_edit(host, cx),
        }
    }
}

/// Takes this device's key for `host` off the relay account, if signed in.
/// Best effort, in the background: forgetting a Host never waits on the
/// control plane.
fn remove_from_account(profile: &AppProfile, host: EndpointKey) {
    let Ok(Some(session)) =
        crate::relay_account::load_session(&crate::relay_account::session_store(profile))
    else {
        return;
    };
    let Ok(secret) = paired_hosts::load_device_key(&paired_hosts::device_key_store(profile), host)
    else {
        return;
    };
    let endpoint_id = yttt_transport_iroh::endpoint_key(&secret).0;
    let _ = std::thread::Builder::new()
        .name("yttt-relay-account".into())
        .spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = runtime.block_on(session.remove_device(&endpoint_id));
            }
        });
}

/// Six digits in two groups, as the Host shows them.
pub(crate) fn format_short_code(code: u32) -> String {
    let digits = format!("{code:06}");
    format!("{} {}", &digits[..3], &digits[3..])
}

/// What the Host's device list will call this computer.
fn device_label() -> String {
    let name = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string();
    let mut label = String::new();
    for character in name.chars() {
        if label.len() + character.len_utf8() > MAX_DEVICE_LABEL_BYTES {
            break;
        }
        label.push(character);
    }
    if label.is_empty() {
        "yttt".into()
    } else {
        label
    }
}

/// Runs the pairing protocol with a fresh key for this Host, and keeps the key
/// and the record only if the Host accepted the device.
fn pair_with_host(
    profile: &AppProfile,
    code: PairingCode,
    name: String,
    events: &flume::Sender<PairEvent>,
    cancelled: flume::Receiver<()>,
) -> Result<PairedHostRecord, String> {
    use yttt_transport_iroh::{DirectPaths, EndpointConfig, IrohEndpoint, OpenGate, RelayTarget};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let secret = paired_hosts::new_device_key();
    let mut code = code;
    let account = crate::relay_account::load_session(&crate::relay_account::session_store(profile))
        .ok()
        .flatten();
    runtime.block_on(async {
        // With a relay account the new key joins the account's relay node,
        // where the account's Host waits; a Host reachable only through that
        // relay can then still be paired.
        let host_name = if name.is_empty() {
            format!("P2P Host {}", &code.host.to_string()[..8])
        } else {
            name.clone()
        };
        let ticket = match &account {
            Some(session) => crate::relay_account::ticket_for(
                session,
                &secret,
                &crate::relay_account::device_label(&host_name),
            )
            .await
            .ok(),
            None => None,
        };
        let relay = match &ticket {
            Some(ticket) => {
                code.relay_url = Some(ticket.relay_url.clone());
                yttt_transport_iroh::RelaySetup::Use(ticket.target())
            }
            None => code
                .relay_url
                .clone()
                .map(|url| RelayTarget { url, token: None })
                .into(),
        };
        let endpoint = IrohEndpoint::bind(EndpointConfig {
            // The key the Host will admit: the work session must use the same one.
            secret: *secret,
            relay,
            alpns: Vec::new(),
            gate: Arc::new(OpenGate),
            direct: DirectPaths::Any,
        })
        .await
        .map_err(|error| format!("cannot open the P2P endpoint: {error}"))?;
        let label = device_label();
        let result = tokio::select! {
            result = yttt_client_core::p2p::pair(&endpoint, &code, &label, |short_code| {
                let _ = events.send(PairEvent::ShortCode(short_code));
            }) => result.map_err(|error| error.to_string()),
            _ = cancelled.recv_async() => Err("pairing was cancelled".to_string()),
        };
        endpoint.close().await;
        let host = match result {
            Ok(host) => host,
            Err(error) => {
                // A key that never paired should not hold one of the plan's
                // device slots.
                if let (Some(session), Some(_)) = (&account, &ticket) {
                    let _ = session
                        .remove_device(&yttt_transport_iroh::endpoint_key(&secret).0)
                        .await;
                }
                return Err(error);
            }
        };
        paired_hosts::save_device_key(
            &paired_hosts::device_key_store(profile),
            host.host,
            &secret,
        )?;
        let record = PairedHostRecord { name, host };
        let mut records = paired_hosts::read_records(profile)?;
        paired_hosts::upsert(&mut records, record.clone())?;
        paired_hosts::save_records(profile, &records)?;
        Ok(record)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_codes_keep_leading_zeros_in_two_groups() {
        assert_eq!(format_short_code(42), "000 042");
        assert_eq!(format_short_code(987_654), "987 654");
    }

    #[test]
    fn the_device_label_fits_the_hosts_limit() {
        let label = device_label();
        assert!(!label.is_empty());
        assert!(label.len() <= MAX_DEVICE_LABEL_BYTES);
    }
}
