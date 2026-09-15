use crate::{
    config::profile::AppProfile,
    remote_launch::{
        ConnectionCode, MAX_CONNECTION_CODE_BYTES, RemoteLaunch, RemoteTarget,
        validate_connection_address,
    },
    ui::{
        i18n::{UiText, UiTextKey},
        primitives::{
            button::{YtttButtonVariant, yttt_button},
            input::{YtttInputKind, yttt_input},
        },
        theme::{current_ui_style, current_workbench_theme},
    },
};
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};
use gpui_component::{
    Disableable as _,
    input::{InputEvent, InputState},
    scroll::ScrollableElement as _,
};
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Write as _};
use yttt_protocol::remote_access::RemoteConnectionInfo;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
struct RememberedConnection {
    address: String,
    environment_id: String,
    credential_id: yttt_core::model::ids::CredentialId,
}

fn read_connections(profile: &AppProfile) -> Result<Vec<RememberedConnection>, String> {
    let file = match std::fs::File::open(
        profile
            .paths()
            .state
            .join("client-connections/existing-host.json"),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(8193)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 8192 {
        return Err("Saved connection metadata exceeds 8 KiB".into());
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Records {
        List(Vec<RememberedConnection>),
        Legacy(RememberedConnection),
    }
    match serde_json::from_slice(&bytes).map_err(|error| error.to_string())? {
        Records::List(records) => Ok(records),
        Records::Legacy(record) => Ok(vec![record]),
    }
}

fn save_connections(profile: &AppProfile, records: &[RememberedConnection]) -> Result<(), String> {
    let bytes = serde_json::to_vec(records).map_err(|error| error.to_string())?;
    if bytes.len() > 8192 {
        return Err("Saved connection metadata exceeds 8 KiB".into());
    }
    let directory = profile.paths().state.join("client-connections");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(&directory).map_err(|error| error.to_string())?;
    temporary
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(directory.join("existing-host.json"))
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn create(
    profile: AppProfile,
    text: UiText,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<ExistingHostForm> {
    let view = cx.new(|cx| {
        let info = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(text.get(UiTextKey::ConnectionCodePlaceholder))
                .validate(|value, _| value.len() <= MAX_CONNECTION_CODE_BYTES)
        });
        let subscription = cx.subscribe_in(
            &info,
            window,
            |view: &mut ExistingHostForm, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.import_code(window, cx);
                }
            },
        );
        ExistingHostForm {
            profile,
            text,
            address: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("host:port")
                    .validate(|value, _| value.len() <= 1024)
            }),
            info,
            remember: false,
            busy: false,
            error: None,
            saved: Vec::new(),
            selected: None,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    });
    view.update(cx, |view, cx| view.load_remembered(window, cx));
    view
}

pub(crate) struct ExistingHostForm {
    profile: AppProfile,
    text: UiText,
    address: Entity<InputState>,
    info: Entity<InputState>,
    remember: bool,
    busy: bool,
    error: Option<String>,
    saved: Vec<RememberedConnection>,
    selected: Option<yttt_core::model::ids::CredentialId>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl ExistingHostForm {
    fn import_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.info.read(cx).value();
        if value.is_empty() {
            self.error = None;
            cx.notify();
            return;
        }
        match ConnectionCode::decode(&value) {
            Ok(code) => {
                self.address
                    .update(cx, |input, cx| input.set_value(code.address, window, cx));
                self.error = None;
            }
            Err(_) => self.error = Some(self.text.get(UiTextKey::ConnectionCodeInvalid).into()),
        }
        cx.notify();
    }
    fn load_remembered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.profile.clone();
        self.busy = true;
        let task = cx.background_spawn(async move { read_connections(&profile) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, _, cx| {
                view.busy = false;
                match result {
                    Ok(saved) => view.saved = saved,
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_saved(
        &mut self,
        record: RememberedConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.error = None;
        let profile = self.profile.clone();
        let task = cx.background_spawn(async move {
            let store = yttt_ssh::CredentialStore::new(format!(
                "{}.existing-host",
                profile.credential_namespace()
            ));
            store
                .load(&record.credential_id)
                .map(|secret| (record, secret))
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok((record, secret)) => {
                        view.selected = Some(record.credential_id);
                        view.remember = true;
                        view.address
                            .update(cx, |input, cx| input.set_value(record.address, window, cx));
                        let payload = secret
                            .as_ref()
                            .and_then(|value| {
                                if ConnectionCode::decode(value).is_ok() {
                                    Some(value.to_string())
                                } else {
                                    // Migrate credentials saved before connection codes included an address.
                                    serde_json::from_str::<RemoteConnectionInfo>(value)
                                        .ok()
                                        .and_then(|info| {
                                            ConnectionCode::encode(
                                                view.address.read(cx).value().to_string(),
                                                info,
                                            )
                                            .ok()
                                        })
                                }
                            })
                            .unwrap_or_default();
                        view.info
                            .update(cx, |input, cx| input.set_value(payload, window, cx));
                        if secret.is_none() {
                            view.error = Some("凭据不可用，请重新粘贴连接信息。".into());
                        }
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let profile = self.profile.clone();
        self.busy = true;
        let task = cx.background_spawn(async move {
            let mut records = read_connections(&profile)?;
            let store = yttt_ssh::CredentialStore::new(format!(
                "{}.existing-host",
                profile.credential_namespace()
            ));
            store.delete(&id).map_err(|error| error.to_string())?;
            records.retain(|record| record.credential_id != id);
            save_connections(&profile, &records)?;
            Ok::<_, String>(records)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(records) => {
                        view.saved = records;
                        view.selected = None;
                        view.remember = false;
                        view.address
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        view.info
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_or_connect(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let address = self.address.read(cx).value().trim().to_string();
        let payload = Zeroizing::new(self.info.read(cx).value().to_string());
        if validate_connection_address(&address).is_err() {
            self.error = Some(self.text.get(UiTextKey::ConnectionAddressInvalid).into());
            cx.notify();
            return;
        }
        let connection_info = match ConnectionCode::decode(&payload) {
            Ok(code) => code.connection_info,
            Err(_) => {
                self.error = Some(self.text.get(UiTextKey::ConnectionCodeInvalid).into());
                cx.notify();
                return;
            }
        };
        let profile = self.profile.clone();
        let remember = self.remember || !connect;
        self.busy = true;
        self.error = None;
        let task = cx.background_spawn(async move {
            if remember {
                let metadata = RememberedConnection { address: address.clone(), environment_id: connection_info.environment_id.clone(), credential_id: yttt_core::model::ids::CredentialId::new(format!("existing-host-{}", connection_info.environment_id)) };
                let store = yttt_ssh::CredentialStore::new(format!("{}.existing-host", profile.credential_namespace()));
                store.save(&metadata.credential_id, &payload).map_err(|error| format!("OS credential store failed; nothing was saved in plaintext. Uncheck Remember to connect once: {error}"))?;
                let mut records = read_connections(&profile)?;
                records.retain(|record| record.environment_id != metadata.environment_id);
                records.push(metadata);
                save_connections(&profile, &records)?;
            }
            if connect {
                crate::remote_launch::spawn_remote_client(RemoteLaunch { local_profile: profile, target: RemoteTarget::ExistingHost { address, connection_info } }).map_err(|error| error.to_string())
            } else {
                Ok(())
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(()) if connect => {
                        view.info
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        view.load_remembered(window, cx);
                    }
                    Ok(()) => {
                        view.remember = true;
                        view.load_remembered(window, cx);
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ExistingHostForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let fields = div()
            .flex()
            .flex_col()
            .gap(style.spacing.md)
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(self.text.get(UiTextKey::ExistingHostDescription)),
            )
            .child(
                div()
                    .id("saved-hosts")
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(style.spacing.xs)
                    .max_h(px(160.0))
                    .overflow_y_scroll()
                    .children(self.saved.iter().enumerate().map(|(index, record)| {
                        let record = record.clone();
                        yttt_button(
                            ("saved-host", index),
                            record.address.clone(),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.select_saved(record.clone(), window, cx)
                            },
                        ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap(style.spacing.sm)
                    .child(
                        yttt_button(
                            "new-saved-host",
                            self.text.get(UiTextKey::SshNewConnection),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.selected = None;
                            view.error = None;
                            view.remember = false;
                            view.address
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            view.info.update(cx, |input, cx| {
                                input.set_value("", window, cx);
                                input.focus(window, cx);
                            });
                            cx.notify();
                        })),
                    )
                    .child(
                        yttt_button(
                            "delete-saved-host",
                            self.text.get(UiTextKey::SshDeleteConnection),
                            YtttButtonVariant::Danger,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy || self.selected.is_none())
                        .on_click(
                            cx.listener(|view, _, window, cx| view.delete_selected(window, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .child(self.text.get(UiTextKey::ConnectionCodeLabel)),
            )
            .child(
                yttt_input(&self.info, YtttInputKind::Settings, theme, style).disabled(self.busy),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(self.text.get(UiTextKey::ConnectionCodeSecurity)),
            )
            .child(
                div()
                    .text_sm()
                    .child(self.text.get(UiTextKey::ConnectionAddressLabel)),
            )
            .child(
                yttt_input(&self.address, YtttInputKind::Settings, theme, style)
                    .disabled(self.busy),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(self.text.get(UiTextKey::ConnectionAddressHint)),
            )
            .children(self.error.clone().map(|error| div().text_sm().child(error)));
        div()
            .debug_selector(|| "existing-host-manager".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.editor_background)
            .text_color(theme.text)
            .child(
                div().flex_1().min_h_0().flex().flex_col().child(
                    div()
                        .id("existing-host-form-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .vertical_scrollbar(&self.scroll)
                        .p(px(24.0))
                        .child(fields),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .p(px(12.0))
                    .gap(style.spacing.sm)
                    .border_t_1()
                    .border_color(theme.border_variant)
                    .child(
                        yttt_button(
                            "existing-host-remember",
                            self.text.get(if self.remember {
                                UiTextKey::ConnectionRemembered
                            } else {
                                UiTextKey::ConnectionRemember
                            }),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.remember = !view.remember;
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(style.spacing.sm)
                            .child(
                                yttt_button(
                                    "save-host-connection",
                                    self.text.get(UiTextKey::SettingsSave),
                                    YtttButtonVariant::Secondary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |view, _, window, cx| view.save_or_connect(false, window, cx),
                                )),
                            )
                            .child(
                                yttt_button(
                                    "existing-host-connect",
                                    self.text.get(UiTextKey::SshConnect),
                                    YtttButtonVariant::Primary,
                                    theme,
                                    style,
                                    cx,
                                )
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    |view, _, window, cx| view.save_or_connect(true, window, cx),
                                )),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn pasting_connection_code_fills_address_and_rejects_bad_credentials(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        use std::{cell::RefCell, rc::Rc};
        cx.update(gpui_component::init);
        let temporary = tempfile::tempdir().unwrap();
        let profile = AppProfile::scoped(
            yttt_core::model::ids::ProfileId::new("paste-test"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            temporary.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        let slot = Rc::new(RefCell::new(None));
        let window_slot = slot.clone();
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let form = create(profile, crate::ui::i18n::UiText::english(), window, cx);
            *window_slot.borrow_mut() = Some(form.clone());
            gpui_component::Root::new(form, window, cx)
        });
        let form = slot.borrow_mut().take().unwrap();
        cx.run_until_parked();
        let info = RemoteConnectionInfo {
            environment_id: "paste-environment".into(),
            profile_id: yttt_core::model::ids::ProfileId::new("paste-profile"),
            server_name: "yttt-host.local".into(),
            certificate_der: vec![1, 2, 3],
            certificate_sha256: "ab".repeat(32),
            credential_generation: 1,
            work_secret: [37; 32],
        };
        let code = ConnectionCode::encode("remote.example:43123".into(), info).unwrap();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(code));
        form.update_in(cx, |form, window, cx| {
            form.info.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-v"
        } else {
            "ctrl-v"
        });
        cx.run_until_parked();
        form.read_with(cx, |form, cx| {
            assert_eq!(
                form.address.read(cx).value().as_str(),
                "remote.example:43123"
            );
            assert!(form.error.is_none());
            assert_eq!(
                ConnectionCode::decode(&form.info.read(cx).value())
                    .unwrap()
                    .connection_info
                    .work_secret,
                [37; 32]
            );
        });
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "not-a-connection-code".into(),
        ));
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a cmd-v"
        } else {
            "ctrl-a ctrl-v"
        });
        cx.run_until_parked();
        form.update_in(cx, |form, window, cx| {
            assert!(form.error.is_some());
            form.save_or_connect(true, window, cx);
            assert!(!form.busy, "invalid credentials must not launch a client");
        });
    }

    #[test]
    fn legacy_connection_survives_adding_another_host() {
        use crate::config::profile::{
            EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        };
        use yttt_core::model::ids::{CredentialId, ProfileId};
        let temporary = tempfile::tempdir().unwrap();
        let profile = AppProfile::scoped(
            ProfileId::new("connections-test"),
            EnvironmentKind::Test,
            ProfilePersistence::Ephemeral,
            temporary.path(),
            ProjectConfigPolicy::Overlay,
            HostConnectPolicy::ProfileDiscovery,
        );
        let directory = profile.paths().state.join("client-connections");
        std::fs::create_dir_all(&directory).unwrap();
        let first = RememberedConnection {
            address: "127.0.0.1:4001".into(),
            environment_id: "first".into(),
            credential_id: CredentialId::new("first-key"),
        };
        std::fs::write(
            directory.join("existing-host.json"),
            serde_json::to_vec(&first).unwrap(),
        )
        .unwrap();
        let mut records = read_connections(&profile).unwrap();
        records.push(RememberedConnection {
            address: "127.0.0.1:4002".into(),
            environment_id: "second".into(),
            credential_id: CredentialId::new("second-key"),
        });
        save_connections(&profile, &records).unwrap();
        let restored = read_connections(&profile).unwrap();
        assert_eq!(
            restored
                .iter()
                .map(|record| record.address.as_str())
                .collect::<Vec<_>>(),
            vec!["127.0.0.1:4001", "127.0.0.1:4002"]
        );
    }
}
