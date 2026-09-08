use super::super::*;

impl WorkbenchView {
    pub(in super::super) fn settings_save_pending(&self) -> bool {
        self.settings.pending_settings_save.is_some() || self.settings.settings_save_in_flight
    }

    /// Pure configuration test backends remain synchronous; every live Host write is staged.
    pub(super) fn persist_app_settings(&mut self, bars_only: bool) -> Result<bool, WorkbenchError> {
        let candidate = std::mem::replace(
            &mut self.app_settings,
            self.settings.confirmed_settings.clone(),
        );
        if self.settings_save_pending() {
            return Err(WorkbenchError::SettingsUnavailable(
                "A settings save is still pending; wait for its result.".into(),
            ));
        }
        if let Some(runtime) = &self.terminal.host_runtime {
            if !runtime.shared_editing_enabled() {
                return Err(WorkbenchError::SettingsUnavailable(
                    "This window does not currently control the shared environment.".into(),
                ));
            }
            self.settings.pending_settings_save = Some((candidate, bars_only));
            return Ok(false);
        }
        if bars_only {
            save_bars(&self.config_paths, &candidate.bars)?;
        } else {
            save_settings(&self.config_paths, &candidate)?;
        }
        self.settings.confirmed_settings = candidate.clone();
        self.app_settings = candidate;
        Ok(true)
    }

    pub(in super::super) fn flush_pending_settings_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((settings, bars_only)) = self.settings.pending_settings_save.take() else {
            return;
        };
        self.settings.settings_save_in_flight = true;
        let paths = self.config_paths.clone();
        let task = cx.background_spawn(async move {
            let theme_store = load_theme_store(&paths).map_err(|error| error.to_string())?;
            let theme = ThemeRuntime::resolve(&settings, &theme_store.store);
            let icons = load_icon_theme(&paths, settings.theme.icon_theme.as_deref())
                .map_err(|error| error.to_string())?;
            if bars_only {
                save_bars(&paths, &settings.bars).map_err(|error| error.to_string())?;
            } else {
                save_settings(&paths, &settings).map_err(|error| error.to_string())?;
            }
            Ok::<_, String>((settings, theme, icons))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.settings.settings_save_in_flight = false;
                match result {
                    Ok((settings, theme, icons)) => {
                        root.settings.settings_save_error = None;
                        root.settings.confirmed_settings = settings.clone();
                        root.app_settings = settings;
                        root.appearance.replace(theme);
                        root.icon_theme = icons;
                        root.ui_text = ui_text_for_language(root.app_settings.general.language);
                        root.system_notifications_enabled = root.app_settings.notifications.system;
                        root.vim.set_support(root.app_settings.vim.mode);
                        root.sync_editor_vim_modes(window, cx);
                        if root.app_settings.vim.mode != VimModeSetting::Global {
                            for pane in root.terminal.terminal_panes.values() {
                                pane.update(cx, |pane, cx| pane.set_terminal_vi_mode(false, cx));
                            }
                        }
                        if root.app_settings.editor.autosave == EditorAutosave::Off {
                            root.project
                                .project_editor_runtime
                                .cancel_all_autosave_tasks();
                        }
                        root.sync_terminal_environment();
                        root.sync_performance_monitoring(cx);
                        root.sync_input_owner_state();
                        root.sync_vim_controller(window, cx);
                        root.settings.keybinding_rows_cache = None;
                        window.set_background_appearance(
                            crate::ui::app::window_background_appearance(
                                root.app_settings.window.effect,
                            ),
                        );
                        root.apply_appearance_change(window, cx);
                    }
                    Err(error) => {
                        root.load_error = Some(format!("Settings were not saved: {error}"));
                        root.settings.settings_save_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
