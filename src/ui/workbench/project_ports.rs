//! The project panel's Ports page: the TCP ports the selected project's
//! terminals listen on, and the ones they usually open.

use std::time::Duration;

use yttt_protocol::ports::{FrequentPort, ListeningPort, ProjectPorts};
use yttt_ui::Icon;
use yttt_ui::menu::{YtttContextMenu as _, YtttMenuItem};

use super::*;

/// How often the visible Ports page asks the Host again; the Host itself
/// scans every few seconds.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

impl WorkbenchView {
    /// The project whose ports the panel shows now, when it shows them.
    fn project_ports_target(&self) -> Option<ProjectId> {
        if self.project.active_panel_page != ProjectPanelPage::Ports
            || !self.selected_project_panel_visible()
            || self.terminal.host_runtime.is_none()
        {
            return None;
        }
        let project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(project_id)?;
        project.location.local_path()?;
        Some(project_id.clone())
    }

    /// Starts or stops asking the Host for ports as the page shows or hides.
    pub(super) fn sync_project_ports_polling(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project_ports_target() else {
            self.project.ports.poll_task = None;
            return;
        };
        let state = &mut self.project.ports;
        if state.project_id.as_ref() != Some(&project_id) {
            *state = ProjectPortsPanelState {
                project_id: Some(project_id.clone()),
                ..ProjectPortsPanelState::default()
            };
        }
        if state.poll_task.is_some() || state.unsupported {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        state.poll_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let request_runtime = runtime.clone();
                let request_project = project_id.clone();
                let result = cx
                    .background_spawn(
                        async move { request_runtime.project_ports(&request_project) },
                    )
                    .await;
                let keep_polling = this
                    .update(cx, |view, cx| {
                        if view.project_ports_target().as_ref() != Some(&project_id) {
                            view.project.ports.poll_task = None;
                            return false;
                        }
                        let state = &mut view.project.ports;
                        let changed = match result {
                            Ok(Some(ports)) => {
                                let changed =
                                    state.ports.as_ref() != Some(&ports) || state.error.is_some();
                                state.ports = Some(ports);
                                state.error = None;
                                changed
                            }
                            Ok(None) => {
                                state.unsupported = true;
                                true
                            }
                            Err(error) => {
                                let changed = state.error.as_ref() != Some(&error);
                                state.error = Some(error);
                                changed
                            }
                        };
                        if changed {
                            cx.notify();
                        }
                        if state.unsupported {
                            state.poll_task = None;
                            return false;
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
        }));
    }

    pub(super) fn project_ports_panel_content(
        &self,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> Div {
        let ssh = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| self.workspace.project(project_id))
            .is_some_and(|project| project.location.local_path().is_none());
        let state = &self.project.ports;
        let message =
            |key: UiTextKey| -> Option<String> { Some(self.ui_text.get(key).to_string()) };
        let notice = if ssh {
            message(UiTextKey::ProjectPortsSshUnavailable)
        } else if state.unsupported {
            message(UiTextKey::ProjectPortsUnsupported)
        } else {
            None
        };
        if let Some(notice) = notice {
            return ports_message("project-ports-notice", notice, theme, ui_style);
        }
        let Some(ports) = state.ports.as_ref() else {
            return match state.error.clone() {
                Some(error) => ports_message("project-ports-error", error, theme, ui_style),
                None => div().flex_1(),
            };
        };
        if ports.listening.is_empty() && ports.frequent.is_empty() {
            return ports_message(
                "project-ports-empty",
                self.ui_text.get(UiTextKey::ProjectPortsEmpty).to_string(),
                theme,
                ui_style,
            );
        }
        let remote = self
            .terminal
            .host_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.is_remote());
        let ProjectPorts {
            listening,
            frequent,
        } = ports.clone();
        let listening_rows = listening
            .iter()
            .map(|port| self.listening_port_row(port, remote, theme, ui_style, cx))
            .collect::<Vec<_>>();
        let frequent_rows = frequent
            .iter()
            .map(|port| self.frequent_port_row(port, theme, ui_style))
            .collect::<Vec<_>>();
        let list = div()
            .debug_selector(|| "project-ports".to_string())
            .id("project-ports-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb(ui_style.spacing.md)
            .child(ports_section_header(
                self.ui_text.get(UiTextKey::ProjectPortsListening),
                listening.len(),
                theme,
                ui_style,
            ))
            .children(listening_rows)
            .when(listening.is_empty(), |list| {
                list.child(
                    div()
                        .px(ui_style.spacing.md)
                        .py(ui_style.spacing.sm)
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(self.ui_text.get(UiTextKey::ProjectPortsEmpty)),
                )
            })
            .when(!frequent.is_empty(), |list| {
                list.child(ports_section_header(
                    self.ui_text.get(UiTextKey::ProjectPortsFrequent),
                    frequent.len(),
                    theme,
                    ui_style,
                ))
                .children(frequent_rows)
            })
            .when(remote && !listening.is_empty(), |list| {
                list.child(
                    div()
                        .debug_selector(|| "project-ports-remote-hint".to_string())
                        .px(ui_style.spacing.md)
                        .pt(ui_style.spacing.md)
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(self.ui_text.get(UiTextKey::ProjectPortsRemoteHint)),
                )
            });
        div().flex().flex_1().min_h_0().child(list)
    }

    fn listening_port_row(
        &self,
        port: &ListeningPort,
        remote: bool,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let url = port_url(port.port);
        let open_url = url.clone();
        let menu_url = url.clone();
        let address = format!("localhost:{}", port.port);
        let port_text = port.port.to_string();
        let open_label = self.ui_text.get(UiTextKey::ProjectPortsOpenInBrowser);
        let copy_address_label = self.ui_text.get(UiTextKey::ProjectPortsCopyAddress);
        let copy_port_label = self.ui_text.get(UiTextKey::ProjectPortsCopyPort);
        let detail = format!("{} · {}", port.process_name, port.addresses.join(", "));
        port_row(
            SharedString::from(format!("project-port-{}", port.port)),
            port.port,
            detail,
            theme.text,
            theme,
            ui_style,
        )
        .when(!remote, |row| {
            row.cursor_pointer()
                .hover(move |style| style.bg(theme.hover_surface))
                .on_click(cx.listener(move |_, _: &ClickEvent, _window, cx| {
                    cx.open_url(&open_url);
                }))
        })
        .yttt_context_menu(
            crate::ui::theme::menu_appearance,
            move |menu, _window, _| {
                let menu_url = menu_url.clone();
                let address = address.clone();
                let port_text = port_text.clone();
                let menu = if remote {
                    menu
                } else {
                    menu.item(
                        YtttMenuItem::new(open_label)
                            .on_click(move |_, _, cx| cx.open_url(&menu_url)),
                    )
                    .item(YtttMenuItem::separator())
                };
                menu.item(
                    YtttMenuItem::new(copy_address_label).on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(address.clone()));
                    }),
                )
                .item(
                    YtttMenuItem::new(copy_port_label).on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(port_text.clone()));
                    }),
                )
            },
        )
    }

    fn frequent_port_row(
        &self,
        port: &FrequentPort,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
    ) -> gpui::Stateful<Div> {
        let detail = format!(
            "{} · {}",
            port.process_name,
            listening_duration_label(port.listening_seconds)
        );
        port_row(
            SharedString::from(format!("project-frequent-port-{}", port.port)),
            port.port,
            detail,
            theme.text_muted,
            theme,
            ui_style,
        )
    }
}

fn port_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

/// A compact total such as `45s`, `12m`, `3h`, or `2d`.
fn listening_duration_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn port_row(
    id: SharedString,
    port: u16,
    detail: String,
    port_color: gpui::Rgba,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .min_w_0()
        .gap(ui_style.spacing.sm)
        .mx(ui_style.spacing.xs)
        .px(ui_style.spacing.sm)
        .py(ui_style.spacing.xs)
        .rounded_sm()
        .text_sm()
        .child(
            Icon::new(gpui_kit_assets::IconName::EthernetPort)
                .size(px(13.0))
                .text_color(theme.icon_muted),
        )
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(port_color)
                .child(port.to_string()),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_xs()
                .text_color(theme.text_subtle)
                .child(detail),
        )
}

fn ports_section_header(
    title: &'static str,
    count: usize,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_none()
        .px(ui_style.spacing.md)
        .pt(ui_style.spacing.md)
        .pb(ui_style.spacing.xs)
        .text_xs()
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .child(title),
        )
        .child(div().text_color(theme.text_subtle).child(count.to_string()))
}

fn ports_message(
    selector: &'static str,
    message: String,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .debug_selector(move || selector.to_string())
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px(ui_style.spacing.xl)
        .text_center()
        .text_sm()
        .text_color(theme.text_subtle)
        .child(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_round_down_to_one_unit() {
        assert_eq!(listening_duration_label(45), "45s");
        assert_eq!(listening_duration_label(60), "1m");
        assert_eq!(listening_duration_label(3_599), "59m");
        assert_eq!(listening_duration_label(7_300), "2h");
        assert_eq!(listening_duration_label(200_000), "2d");
    }

    #[test]
    fn a_port_opens_on_localhost() {
        assert_eq!(port_url(5173), "http://localhost:5173");
    }
}
