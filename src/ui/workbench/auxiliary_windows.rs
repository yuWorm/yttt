use gpui::{AnyWindowHandle, App, Bounds, WeakEntity, size};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuxiliaryWindowKind {
    Settings,
    RemoteServices,
    LayoutEditor,
}

#[derive(Default)]
struct WindowSlot {
    window: Option<AnyWindowHandle>,
    opening: bool,
    focus_requested: bool,
}

#[derive(Default)]
pub(super) struct AuxiliaryWindows {
    settings: WindowSlot,
    remote_services: WindowSlot,
    layout_editor: WindowSlot,
    pub(super) remote_access_page: bool,
    pub(super) active: Option<AuxiliaryWindowKind>,
}

impl AuxiliaryWindows {
    fn slot(&mut self, kind: AuxiliaryWindowKind) -> &mut WindowSlot {
        match kind {
            AuxiliaryWindowKind::Settings => &mut self.settings,
            AuxiliaryWindowKind::RemoteServices => &mut self.remote_services,
            AuxiliaryWindowKind::LayoutEditor => &mut self.layout_editor,
        }
    }

    pub(super) fn request(&mut self, kind: AuxiliaryWindowKind) {
        self.slot(kind).focus_requested = true;
        self.active = Some(kind);
    }
}

impl WorkbenchView {
    pub(super) fn is_foreground_keyboard_window(&self, window: &Window) -> bool {
        let handle = Some(window.window_handle());
        match self.auxiliary_windows.active {
            Some(AuxiliaryWindowKind::Settings) => self.auxiliary_windows.settings.window == handle,
            Some(AuxiliaryWindowKind::RemoteServices) => {
                self.auxiliary_windows.remote_services.window == handle
            }
            Some(AuxiliaryWindowKind::LayoutEditor) => {
                self.auxiliary_windows.layout_editor.window == handle
            }
            None => {
                self.auxiliary_windows.settings.window != handle
                    && self.auxiliary_windows.remote_services.window != handle
                    && self.auxiliary_windows.layout_editor.window != handle
            }
        }
    }

    pub(super) fn settings_dialogs_are_foreground(&self) -> bool {
        match self.auxiliary_windows.active {
            Some(AuxiliaryWindowKind::Settings) => true,
            Some(AuxiliaryWindowKind::RemoteServices | AuxiliaryWindowKind::LayoutEditor) => false,
            None => !self.settings.settings_page.is_open,
        }
    }

    fn auxiliary_window_is_open(&self, kind: AuxiliaryWindowKind) -> bool {
        match kind {
            AuxiliaryWindowKind::Settings => self.settings.settings_page.is_open,
            AuxiliaryWindowKind::RemoteServices => self.ssh.manager_open,
            AuxiliaryWindowKind::LayoutEditor => self.layout_toml_editor_is_open(),
        }
    }

    fn close_auxiliary_window(&mut self, kind: AuxiliaryWindowKind) {
        match kind {
            AuxiliaryWindowKind::Settings => self.close_settings(),
            AuxiliaryWindowKind::RemoteServices => self.close_ssh_connection_manager(),
            AuxiliaryWindowKind::LayoutEditor => self.cancel_layout_toml_editor(),
        }
    }

    pub(super) fn sync_auxiliary_windows(&mut self, cx: &mut Context<Self>) {
        for kind in [
            AuxiliaryWindowKind::Settings,
            AuxiliaryWindowKind::RemoteServices,
            AuxiliaryWindowKind::LayoutEditor,
        ] {
            let is_open = self.auxiliary_window_is_open(kind);
            let slot = self.auxiliary_windows.slot(kind);
            if !is_open {
                if let Some(handle) = slot.window.take() {
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    });
                }
                continue;
            }
            if let Some(handle) = slot.window {
                if std::mem::take(&mut slot.focus_requested) {
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, _| window.activate_window());
                    });
                }
            } else if !slot.opening {
                slot.opening = true;
                let owner = cx.weak_entity();
                // Window creation must happen after the workbench's entity borrow ends.
                cx.defer(move |cx| open_auxiliary_window(owner, kind, cx));
            }
        }
    }
}

fn open_auxiliary_window(
    owner: WeakEntity<WorkbenchView>,
    kind: AuxiliaryWindowKind,
    cx: &mut App,
) {
    let Some(owner) = owner.upgrade() else { return };
    if !owner.read(cx).auxiliary_window_is_open(kind) {
        owner.update(cx, |root, _| {
            root.auxiliary_windows.slot(kind).opening = false
        });
        return;
    }
    let appearance = owner.read(cx).theme_runtime();
    let scale = appearance.typography.font_size / 16.0;
    let dimensions = match kind {
        AuxiliaryWindowKind::Settings => size(px(1080.0 * scale), px(760.0 * scale)),
        AuxiliaryWindowKind::RemoteServices => size(px(960.0 * scale), px(720.0 * scale)),
        AuxiliaryWindowKind::LayoutEditor => size(px(960.0 * scale), px(720.0 * scale)),
    };
    let bounds = Bounds::centered(None, dimensions, cx);
    let mut options =
        crate::ui::app::workbench_window_options(bounds, owner.read(cx).app_settings.window.effect);
    options.window_min_size = Some(match kind {
        AuxiliaryWindowKind::Settings => size(px(900.0), px(480.0)),
        AuxiliaryWindowKind::RemoteServices => size(px(720.0), px(480.0)),
        AuxiliaryWindowKind::LayoutEditor => size(px(560.0), px(360.0)),
    });
    let window_owner = owner.clone();
    let result = cx.open_window(options, move |window, cx| {
        window_owner.update(cx, |root, _| {
            if kind == AuxiliaryWindowKind::Settings {
                root.reset_settings_search_input();
                root.settings.settings_search_input_needs_focus = true;
            }
        });
        let view = cx.new(|cx| AuxiliaryWindow::new(&window_owner, kind, window, cx));
        let weak_owner = window_owner.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak_owner.update(cx, |root, cx| {
                root.flush_pending_settings_save(window, cx);
                root.close_auxiliary_window(kind);
                cx.notify();
            });
            true
        });
        window.activate_window();
        cx.new(|cx| ComponentRoot::new(view, window, cx))
    });
    owner.update(cx, |root, cx| {
        let slot = root.auxiliary_windows.slot(kind);
        slot.opening = false;
        slot.focus_requested = false;
        match result {
            Ok(handle) => slot.window = Some(handle.into()),
            Err(error) => {
                root.close_auxiliary_window(kind);
                root.load_error = Some(format!("Failed to open window: {error}"));
            }
        }
        cx.notify();
    });
}

struct AuxiliaryWindow {
    owner: WeakEntity<WorkbenchView>,
    kind: AuxiliaryWindowKind,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AuxiliaryWindow {
    fn new(
        owner: &Entity<WorkbenchView>,
        kind: AuxiliaryWindowKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let handle = window.window_handle();
        let weak_owner = owner.downgrade();
        let subscriptions = vec![
            cx.observe(owner, |_, _, cx| cx.notify()),
            cx.observe_release_in(owner, window, |_, _, window, _| window.remove_window()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    let _ = this.owner.update(cx, |root, cx| {
                        root.auxiliary_windows.active = Some(this.kind);
                        root.sync_input_owner_state();
                        if this.kind == AuxiliaryWindowKind::Settings {
                            root.refresh_permission_statuses(cx);
                            root.refresh_login_startup(cx);
                        }
                        cx.notify();
                    });
                }
            }),
            cx.on_release(move |_, cx| {
                let _ = weak_owner.update(cx, |root, cx| {
                    let slot = root.auxiliary_windows.slot(kind);
                    if slot.window == Some(handle) {
                        slot.window = None;
                        root.close_auxiliary_window(kind);
                        cx.notify();
                    }
                });
            }),
        ];
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            owner: owner.downgrade(),
            kind,
            focus_handle,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for AuxiliaryWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            window.remove_window();
            return div();
        };
        if !owner.read(cx).auxiliary_window_is_open(self.kind) {
            window.remove_window();
            return div();
        }
        let kind = self.kind;
        let content = owner.update(cx, |root, cx| {
            root.flush_pending_settings_save(window, cx);
            root.flush_pending_status_notifications(window, cx);
            let appearance = root.theme_runtime();
            window.set_rem_size(px(appearance.typography.font_size));
            root.sync_vim_controller(window, cx);
            let title = match kind {
                AuxiliaryWindowKind::Settings => {
                    root.ui_text.get(UiTextKey::SettingsWindowTitle).to_string()
                }
                AuxiliaryWindowKind::RemoteServices => {
                    root.ui_text.get(UiTextKey::RemoteServices).to_string()
                }
                AuxiliaryWindowKind::LayoutEditor => root
                    .layout_toml_editor_path()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            };
            window.set_window_title(&format!("{title} — yttt"));
            let body = match kind {
                AuxiliaryWindowKind::Settings => root
                    .settings_search_input(window, cx)
                    .map(|input| settings_window_content(root, &input, window, cx))
                    .unwrap_or_else(div),
                AuxiliaryWindowKind::RemoteServices => {
                    remote_services_window_content(root, window, cx)
                }
                AuxiliaryWindowKind::LayoutEditor => root
                    .layout_toml_input(window, cx)
                    .map(|input| render::layout_toml_editor_window_content(root, &input, cx))
                    .unwrap_or_else(div),
            };
            let mut content = div()
                .debug_selector(move || match kind {
                    AuxiliaryWindowKind::Settings => "settings-window".into(),
                    AuxiliaryWindowKind::RemoteServices => "remote-services-window".into(),
                    AuxiliaryWindowKind::LayoutEditor => "layout-editor-window".into(),
                })
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .overflow_hidden()
                .bg(appearance.ui.app_background)
                .text_color(appearance.ui.text)
                .font_family(appearance.typography.font_family.clone())
                .line_height(relative(appearance.typography.line_height))
                .child(workbench_titlebar(
                    shell::bar::BarSections {
                        left: vec![div().child(title).into_any_element()],
                        ..Default::default()
                    },
                    appearance.ui,
                    appearance.style,
                    window,
                ))
                .child(body.flex_1().min_h_0());
            if kind == AuxiliaryWindowKind::Settings {
                if let Some(dialog) = root.settings.zed_theme_import_dialog.clone() {
                    content = content.child(zed_theme_import_dialog(
                        cx,
                        &root.ui_text,
                        &dialog.detection,
                        &dialog.existing_paths,
                        dialog.conflict_policy,
                        &root.config_paths,
                        appearance.ui,
                    ));
                }
                content = root.render_keybinding_dialog(content, &self.focus_handle, window, cx);
            }
            if kind == AuxiliaryWindowKind::RemoteServices && !root.ssh.pending_host_keys.is_empty()
            {
                content = content.child(ssh_host_key_overlay(root, cx));
            }
            let mut key_context = if root.overlays.pending_keybinding_edit.is_some() {
                gpui::KeyContext::new_with_defaults()
            } else {
                root.vim.current_key_context()
            };
            key_context.add("YtttAuxiliaryWindow");
            if root.overlays.pending_keybinding_edit.is_some() {
                key_context.add("YtttKeybindingRecorder");
            } else {
                key_context.add(WORKSPACE_CONTEXT);
            }
            let focus_handle = self.focus_handle.clone();
            content
                .key_context(key_context)
                .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                    if kind == AuxiliaryWindowKind::Settings
                        && root.overlays.pending_keybinding_edit.is_some()
                    {
                        root.on_key_down(event, window, cx);
                        return;
                    }
                    if kind == AuxiliaryWindowKind::LayoutEditor
                        && event.keystroke.key == "s"
                        && if cfg!(target_os = "macos") {
                            event.keystroke.modifiers.platform
                        } else {
                            event.keystroke.modifiers.control
                        }
                    {
                        let _ = root.save_layout_toml_editor();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    let close_shortcut = event.keystroke.key == "w"
                        && if cfg!(target_os = "macos") {
                            event.keystroke.modifiers.platform
                        } else {
                            event.keystroke.modifiers.control
                        };
                    if close_shortcut {
                        root.close_auxiliary_window(kind);
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if event.keystroke.key == "escape" && kind != AuxiliaryWindowKind::LayoutEditor
                    {
                        if kind == AuxiliaryWindowKind::Settings
                            && root.vim.support() == VimModeSetting::Global
                            && root.settings_text_input_is_focused(window, cx)
                        {
                            focus_handle.focus(window, cx);
                            root.sync_vim_controller(window, cx);
                            root.vim.enter_normal();
                        } else if kind == AuxiliaryWindowKind::Settings
                            && root.zed_theme_import_dialog_is_open()
                        {
                            root.cancel_zed_theme_import_dialog();
                        } else {
                            root.close_auxiliary_window(kind);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(move |root, _: &TabClose, _, cx| {
                    root.close_auxiliary_window(kind);
                    cx.notify();
                }))
                .on_action(cx.listener(WorkbenchView::on_application_quit))
                .on_action(cx.listener(WorkbenchView::on_settings_open))
                .on_action(cx.listener(WorkbenchView::on_settings_notifications))
                .on_action(cx.listener(WorkbenchView::on_settings_vim_previous_group))
                .on_action(cx.listener(WorkbenchView::on_settings_vim_next_group))
                .on_action(cx.listener(WorkbenchView::on_settings_vim_first_group))
                .on_action(cx.listener(WorkbenchView::on_settings_vim_last_group))
                .on_action(cx.listener(WorkbenchView::on_vim_enter_normal))
                .on_action(cx.listener(WorkbenchView::on_vim_enter_insert))
        });
        let mut content = content.track_focus(&self.focus_handle);
        if let Some(layer) = ComponentRoot::render_notification_layer(window, cx) {
            content = content.child(layer);
        }
        if let Some(layer) = ComponentRoot::render_sheet_layer(window, cx) {
            content = content.child(layer);
        }
        if let Some(layer) = ComponentRoot::render_dialog_layer(window, cx) {
            content = content.child(layer);
        }
        content
    }
}
