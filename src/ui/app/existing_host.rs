use crate::{
    config::profile::AppProfile,
    remote_launch::{RemoteLaunch, RemoteTarget},
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
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::{Disableable as _, Root, input::InputState, scroll::ScrollableElement as _};
use serde::{Deserialize, Serialize};
use std::io::{Read as _, Write as _};
use yttt_protocol::remote_access::{MAX_CONNECTION_INFO_BYTES, RemoteConnectionInfo};
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

pub(crate) fn open(profile: AppProfile, text: UiText, cx: &mut App) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(620.0), px(620.0)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        move |window, cx| {
            window.set_window_title(text.get(UiTextKey::ConnectExistingHost));
            let view = cx.new(|cx| ExistingHostForm {
                profile,
                text,
                address: cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("127.0.0.1:43123")
                        .validate(|value, _| value.len() <= 1024)
                }),
                info: cx.new(|cx| {
                    InputState::new(window, cx)
                        .masked(true)
                        .placeholder("Connection information / 连接信息 (≤ 8 KiB)")
                        .validate(|value, _| value.len() <= MAX_CONNECTION_INFO_BYTES)
                }),
                remember: false,
                busy: false,
                error: None,
                saved: Vec::new(),
                selected: None,
            });
            view.update(cx, |view, cx| view.load_remembered(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    )?;
    Ok(())
}

struct ExistingHostForm {
    profile: AppProfile,
    text: UiText,
    address: Entity<InputState>,
    info: Entity<InputState>,
    remember: bool,
    busy: bool,
    error: Option<String>,
    saved: Vec<RememberedConnection>,
    selected: Option<yttt_core::model::ids::CredentialId>,
}

impl ExistingHostForm {
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
                        view.info.update(cx, |input, cx| {
                            input.set_value(
                                secret
                                    .as_ref()
                                    .map(|value| value.to_string())
                                    .unwrap_or_default(),
                                window,
                                cx,
                            )
                        });
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
        if payload.len() > MAX_CONNECTION_INFO_BYTES {
            self.error = Some("Connection information exceeds 8 KiB".into());
            cx.notify();
            return;
        }
        if address.is_empty()
            || address.len() > 1024
            || address.contains(['/', '\\', '\n', '\r', ' '])
            || !address.contains(':')
        {
            self.error = Some("Enter a forwarded host:port or [IPv6]:port, not a URL.".into());
            cx.notify();
            return;
        }
        let connection_info: RemoteConnectionInfo = match serde_json::from_str(&payload) {
            Ok(info) => info,
            Err(_) => {
                self.error = Some("Invalid connection information. Copy it again from the target computer's remote-access settings.".into());
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
                        window.remove_window();
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
        div()
            .size_full()
            .flex()
            .flex_col()
            .p(px(24.0))
            .gap(style.spacing.md)
            .bg(theme.panel_background)
            .text_color(theme.text)
            .child(div().child(self.text.get(UiTextKey::ConnectExistingHost)))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child("TLS · No SSH or Server deployment / 无需 SSH 或部署 Server"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(style.spacing.xs)
                    .max_h(px(160.0))
                    .overflow_y_scrollbar()
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
                            "新建连接",
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
                            view.info
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            cx.notify();
                        })),
                    )
                    .child(
                        yttt_button(
                            "delete-saved-host",
                            "删除选中的连接",
                            YtttButtonVariant::Secondary,
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
                yttt_input(&self.address, YtttInputKind::Settings, theme, style)
                    .disabled(self.busy),
            )
            .child(
                yttt_input(&self.info, YtttInputKind::Settings, theme, style).disabled(self.busy),
            )
            .child(
                div()
                    .flex()
                    .gap(style.spacing.sm)
                    .child(
                        yttt_button(
                            "existing-host-remember",
                            if self.remember {
                                "Remember: OS keychain / 已记住"
                            } else {
                                "Remember credentials / 记住凭据"
                            },
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
                        yttt_button(
                            "save-host-connection",
                            "保存",
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.save_or_connect(false, window, cx)
                        })),
                    )
                    .child(
                        yttt_button(
                            "existing-host-connect",
                            self.text.get(UiTextKey::ConnectExistingHost),
                            YtttButtonVariant::Primary,
                            theme,
                            style,
                            cx,
                        )
                        .disabled(self.busy)
                        .on_click(
                            cx.listener(|view, _, window, cx| {
                                view.save_or_connect(true, window, cx)
                            }),
                        ),
                    ),
            )
            .children(self.error.clone().map(|error| div().text_sm().child(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
