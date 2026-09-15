use super::auxiliary_windows::RemoteServicesPage;
use super::*;
use crate::ui::theme::current_workbench_theme;
use gpui_component::{
    FocusTrapElement as _,
    menu::{DropdownMenu as _, PopupMenuItem},
};
use yttt_core::model::ids::{ConnectionId, CredentialId};
use yttt_ui::primitives::button::{yttt_button, yttt_button_base};

#[derive(Clone)]
enum Target {
    Ssh(ConnectionId),
    Host(CredentialId),
}

impl WorkbenchView {
    fn ensure_existing_hosts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.auxiliary_windows.existing_host.is_none() {
            if let Some(profile) = self.config_paths.profile().cloned() {
                let hosts =
                    crate::ui::app::existing_host::create(profile, self.ui_text, window, cx);
                self.auxiliary_windows.existing_host_subscription =
                    Some(cx.observe(&hosts, |_, _, cx| cx.notify()));
                self.auxiliary_windows.existing_host = Some(hosts);
            }
        }
        if self.auxiliary_windows.pending_new_host_editor
            && self
                .auxiliary_windows
                .existing_host
                .as_ref()
                .is_some_and(|hosts| !hosts.read(cx).busy())
        {
            self.auxiliary_windows.pending_new_host_editor = false;
            self.new_remote_host(window, cx);
        }
    }

    fn new_remote_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_ssh_connection_editor();
        if let Some(hosts) = self.auxiliary_windows.existing_host.clone() {
            hosts.update(cx, |hosts, cx| hosts.new_connection(window, cx));
        }
        cx.notify();
    }

    pub(super) fn remote_connection_editor_open(&self, cx: &App) -> bool {
        self.ssh.editor_open
            || self
                .auxiliary_windows
                .existing_host
                .as_ref()
                .is_some_and(|hosts| hosts.read(cx).editor_open())
    }

    pub(super) fn dismiss_remote_connection_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_ssh_connection_editor();
        if let Some(hosts) = self.auxiliary_windows.existing_host.clone() {
            hosts.update(cx, |hosts, cx| hosts.dismiss_editor(window, cx));
        }
        if let Some(focus) = &self.auxiliary_windows.remote_manager_focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn connect_remote_record(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Ssh(id) => self.connect_ssh_connection(id, window, cx),
            Target::Host(id) => {
                if let Some(hosts) = self.auxiliary_windows.existing_host.clone() {
                    hosts.update(cx, |hosts, cx| hosts.connect_saved(id, window, cx));
                }
            }
        }
    }

    fn edit_remote_record(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Ssh(id) => self.open_ssh_connection_editor(Some(id), window, cx),
            Target::Host(id) => {
                if let Some(hosts) = self.auxiliary_windows.existing_host.clone() {
                    hosts.update(cx, |hosts, cx| hosts.edit_connection(id, window, cx));
                }
            }
        }
    }

    fn delete_remote_record(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Ssh(id) => self.delete_ssh_connection(id, window, cx),
            Target::Host(id) => {
                if let Some(hosts) = self.auxiliary_windows.existing_host.clone() {
                    hosts.update(cx, |hosts, cx| hosts.delete_connection(id, window, cx));
                }
            }
        }
    }
}

pub(super) fn render(
    root: &mut WorkbenchView,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    root.ensure_existing_hosts(window, cx);
    let theme = root.theme_runtime().ui;
    let style = current_ui_style(cx);
    let manager_focus = root
        .auxiliary_windows
        .remote_manager_focus
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    let navigation = div()
        .flex()
        .flex_none()
        .items_center()
        .px(gpui::rems(1.0))
        .gap(style.spacing.lg)
        .border_b_1()
        .border_color(theme.border_variant)
        .children(
            [
                (
                    RemoteServicesPage::Connections,
                    "remote-services-connections",
                    UiTextKey::RemoteConnections,
                ),
                (
                    RemoteServicesPage::ThisComputer,
                    "remote-services-this-computer",
                    UiTextKey::RemoteAccessTitle,
                ),
            ]
            .into_iter()
            .map(|(page, id, key)| {
                let selected = root.auxiliary_windows.remote_page == page;
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .py(gpui::rems(0.625))
                    .border_b(px(2.0))
                    .border_color(if selected {
                        theme.text_muted
                    } else {
                        theme.text_muted.alpha(0.0)
                    })
                    .text_sm()
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .cursor_pointer()
                    .child(root.ui_text.get(key))
                    .on_click(cx.listener(move |root, _, _, cx| {
                        root.auxiliary_windows.remote_page = page;
                        cx.notify();
                    }))
            }),
        );
    let body = match root.auxiliary_windows.remote_page {
        RemoteServicesPage::Connections => connections(root, cx),
        RemoteServicesPage::ThisComputer => div().flex().flex_col().flex_1().min_h_0().child(
            div()
                .id("remote-access-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .vertical_scrollbar(&root.auxiliary_windows.remote_access_scroll)
                .p(gpui::rems(2.0))
                .child(root.remote_access_settings(window, cx)),
        ),
    };
    let mut view = div()
        .debug_selector(|| "remote-services-manager".into())
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .overflow_hidden()
        .bg(theme.editor_background)
        .track_focus(&manager_focus)
        .child(navigation)
        .child(body);
    if root.remote_connection_editor_open(cx) {
        let focus = root
            .auxiliary_windows
            .remote_editor_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let credentials_only = root.ssh.editor_open && root.ssh.credentials_only
            || !root.ssh.editor_open
                && root
                    .auxiliary_windows
                    .existing_host
                    .as_ref()
                    .is_some_and(|hosts| hosts.read(cx).credentials_only());
        let (title, editor) = if root.ssh.editor_open {
            let title = root.ui_text.get(if root.ssh.credentials_only {
                UiTextKey::RemoteCredentialsRequired
            } else {
                UiTextKey::SshConnections
            });
            (
                title,
                ssh_connections::ssh_connection_editor(root, window, cx).into_any_element(),
            )
        } else {
            (
                root.ui_text.get(if credentials_only {
                    UiTextKey::RemoteCredentialsRequired
                } else {
                    UiTextKey::RemoteAddHost
                }),
                root.auxiliary_windows
                    .existing_host
                    .as_ref()
                    .unwrap()
                    .clone()
                    .into_any_element(),
            )
        };
        let preferred_height = if credentials_only {
            px(320.0)
        } else {
            px(620.0)
        };
        let height = (window.viewport_size().height - px(120.0))
            .max(px(200.0))
            .min(preferred_height);
        let panel = yttt_dialog_surface(theme, style)
            .debug_selector(|| "remote-connection-editor".into())
            .w(if credentials_only {
                px(520.0)
            } else {
                px(680.0)
            })
            .max_w(relative(0.94))
            .h(height)
            .max_h(relative(0.94))
            .p_0()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .px(px(20.0))
                    .py(px(14.0))
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().flex_1().min_h_0().child(editor))
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    root.dismiss_remote_connection_editor(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(
                cx.listener(|root, _: &gpui_component::input::Escape, window, cx| {
                    root.dismiss_remote_connection_editor(window, cx);
                    cx.stop_propagation();
                }),
            )
            .focus_trap("remote-connection-editor-trap", &focus);
        view = view.child(yttt_dialog_overlay(
            panel,
            YtttDialogPlacement::Center,
            theme,
            style,
        ));
    }
    view
}

fn connections(root: &WorkbenchView, cx: &mut Context<WorkbenchView>) -> Div {
    let theme = root.theme_runtime().ui;
    let style = current_ui_style(cx);
    let text = root.ui_text;
    let owner = cx.weak_entity();
    let add = yttt_button(
        "remote-add",
        text.get(UiTextKey::RemoteAdd),
        YtttButtonVariant::Primary,
        theme,
        style,
        cx,
    )
    .dropdown_menu(move |menu, _, cx| {
        let ssh_owner = owner.clone();
        let host_owner = owner.clone();
        yttt_ui::primitives::menu::yttt_popup_menu(
            menu,
            current_workbench_theme(cx),
            current_ui_style(cx),
        )
        .item(
            PopupMenuItem::new(text.get(UiTextKey::RemoteAddSsh)).on_click(move |_, window, cx| {
                let _ = ssh_owner.update(cx, |root, cx| {
                    root.open_ssh_connection_editor(None, window, cx)
                });
            }),
        )
        .item(
            PopupMenuItem::new(text.get(UiTextKey::RemoteAddHost)).on_click(
                move |_, window, cx| {
                    let _ = host_owner.update(cx, |root, cx| root.new_remote_host(window, cx));
                },
            ),
        )
    });
    let mut rows = Vec::new();
    for connection in &root.ssh.connections.connections {
        rows.push(connection_row(
            root,
            Target::Ssh(connection.id.clone()),
            connection.name.clone(),
            format!(
                "{}@{}:{}",
                connection.user, connection.host, connection.port
            ),
            "SSH",
            root.ssh.connecting.is_some(),
            cx,
        ));
    }
    if let Some(hosts) = root.auxiliary_windows.existing_host.as_ref() {
        let records = hosts.read(cx).records().to_vec();
        let busy = hosts.read(cx).busy();
        for record in records {
            let name = if record.name.trim().is_empty() {
                record.address.clone()
            } else {
                record.name
            };
            rows.push(connection_row(
                root,
                Target::Host(record.credential_id),
                name,
                record.address,
                "Host",
                busy,
                cx,
            ));
        }
    }
    let empty = rows.is_empty();
    let host_error = root
        .auxiliary_windows
        .existing_host
        .as_ref()
        .and_then(|hosts| hosts.read(cx).error())
        .map(str::to_owned);
    let error = if root.remote_connection_editor_open(cx) {
        None
    } else {
        root.ssh.error.clone().or(host_error)
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex()
                .flex_none()
                .justify_end()
                .p(gpui::rems(1.0))
                .child(add),
        )
        .child(
            div().flex().flex_col().flex_1().min_h_0().child(
                div()
                    .id("remote-connections-list")
                    .debug_selector(|| "remote-connections-list".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .vertical_scrollbar(&root.auxiliary_windows.remote_list_scroll)
                    .px(gpui::rems(1.0))
                    .pb(gpui::rems(1.0))
                    .child(div().flex().flex_col().gap(style.spacing.sm).children(rows))
                    .when(empty, |view| {
                        view.child(
                            div()
                                .p(gpui::rems(2.0))
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child(text.get(UiTextKey::RemoteEmpty)),
                        )
                    }),
            ),
        )
        .children(error.map(|error| div().flex_none().p(gpui::rems(1.0)).text_sm().child(error)))
}

fn connection_row(
    root: &WorkbenchView,
    target: Target,
    name: String,
    address: String,
    kind: &'static str,
    busy: bool,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let style = current_ui_style(cx);
    let text = root.ui_text;
    let id = match &target {
        Target::Ssh(id) => format!("ssh-{}", id),
        Target::Host(id) => format!("host-{}", id),
    };
    let debug_id = format!("remote-record-{id}");
    let menu_owner = cx.weak_entity();
    let menu_target = target.clone();
    let options = yttt_button(
        SharedString::from(format!("remote-options-{id}")),
        "⋯",
        YtttButtonVariant::Ghost,
        theme,
        style,
        cx,
    )
    .disabled(busy)
    .dropdown_menu(move |menu, _, cx| {
        let edit_owner = menu_owner.clone();
        let delete_owner = menu_owner.clone();
        let edit_target = menu_target.clone();
        let delete_target = menu_target.clone();
        yttt_ui::primitives::menu::yttt_popup_menu(
            menu,
            current_workbench_theme(cx),
            current_ui_style(cx),
        )
        .item(
            PopupMenuItem::new(text.get(UiTextKey::RemoteEdit)).on_click(move |_, window, cx| {
                let _ = edit_owner.update(cx, |root, cx| {
                    root.edit_remote_record(edit_target.clone(), window, cx)
                });
            }),
        )
        .item(
            PopupMenuItem::new(text.get(UiTextKey::SshDeleteConnection)).on_click(
                move |_, window, cx| {
                    let _ = delete_owner.update(cx, |root, cx| {
                        root.delete_remote_record(delete_target.clone(), window, cx)
                    });
                },
            ),
        )
    });
    div()
        .debug_selector(move || debug_id.clone())
        .flex()
        .items_center()
        .gap(style.spacing.sm)
        .border_b_1()
        .border_color(theme.border_variant)
        .child(
            yttt_button_base(
                SharedString::from(format!("remote-connect-{id}")),
                YtttButtonVariant::Ghost,
                theme,
                style,
                cx,
            )
            .on_click(cx.listener(move |root, _, window, cx| {
                root.connect_remote_record(target.clone(), window, cx)
            }))
            .disabled(busy)
            .flex_1()
            .min_w_0()
            .h_auto()
            .justify_start()
            .py(px(12.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .min_w_0()
                    .gap(style.spacing.lg)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(style.spacing.xs)
                            .child(div().text_sm().truncate().child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .truncate()
                                    .child(address),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(kind),
                    ),
            ),
        )
        .child(options)
}
