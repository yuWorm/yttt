use super::super::layout_editor_controller::validate_bars_editor_source;
use super::super::*;
use crate::config::scope::SettingsScope;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectSettingsRecoveryKind {
    Project,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectSettingsRecoveryCandidate {
    kind: ProjectSettingsRecoveryKind,
    project_id: String,
    project_path: std::path::PathBuf,
    key: crate::config::project_settings::ProjectEditorSettingKey,
    candidate: Option<crate::config::project_settings::ProjectEditorSettingValue>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ProjectSettingsRecoveryConfirmed {
    kind: ProjectSettingsRecoveryKind,
    project_id: String,
    project_path: std::path::PathBuf,
    key: crate::config::project_settings::ProjectEditorSettingKey,
    confirmed: crate::config::project_settings::EffectiveProjectEditorSetting,
}

fn project_settings_recovery_values(
    draft: &ProjectSettingsSaveDraft,
) -> (serde_json::Value, serde_json::Value) {
    (
        serde_json::to_value(ProjectSettingsRecoveryCandidate {
            kind: ProjectSettingsRecoveryKind::Project,
            project_id: draft.project_id.clone(),
            project_path: draft.project_path.clone(),
            key: draft.key,
            candidate: draft.candidate.clone(),
        })
        .expect("project settings recovery candidate is JSON serializable"),
        serde_json::to_value(ProjectSettingsRecoveryConfirmed {
            kind: ProjectSettingsRecoveryKind::Project,
            project_id: draft.project_id.clone(),
            project_path: draft.project_path.clone(),
            key: draft.key,
            confirmed: draft.confirmed.clone(),
        })
        .expect("project settings recovery baseline is JSON serializable"),
    )
}

fn project_editor_defaults_changed(current: &AppSettings, next: &AppSettings) -> bool {
    current.editor.tab_size != next.editor.tab_size
        || current.editor.auto_detect_language != next.editor.auto_detect_language
        || current.editor.default_language != next.editor.default_language
}

impl WorkbenchView {
    pub(in super::super) fn settings_save_pending(&self) -> bool {
        self.settings.pending_settings_save.is_some()
            || self.settings.settings_save_in_flight
            || self.settings.pending_project_settings_save.is_some()
            || self.settings.project_settings_save_in_flight
            || self.pending_onboarding_completion.is_some()
            || self.onboarding_completion_in_flight
    }

    pub(in super::super) fn has_failed_settings_save(&self) -> bool {
        self.failed_settings_save.is_some() || self.settings.failed_project_settings_save.is_some()
    }

    pub(super) fn copy_failed_settings_draft(&mut self, cx: &mut Context<Self>) {
        let source = if let Some(draft) = self.settings.failed_project_settings_save.as_ref() {
            let (candidate, _) = project_settings_recovery_values(draft);
            serde_json::to_string_pretty(&candidate).map_err(|error| error.to_string())
        } else {
            let Some((candidate, _, bars_only)) = self.failed_settings_save.as_ref() else {
                return;
            };
            if *bars_only {
                toml::to_string_pretty(&candidate.bars).map_err(|error| error.to_string())
            } else {
                toml::to_string_pretty(candidate).map_err(|error| error.to_string())
            }
        };
        match source {
            Ok(source) => cx.write_to_clipboard(ClipboardItem::new_string(source)),
            Err(error) => {
                self.load_error = Some(format!("Settings draft could not be copied: {error}"))
            }
        }
    }

    pub(super) fn discard_failed_settings_save(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.settings.failed_project_settings_save.take() {
            let (candidate, confirmed) = project_settings_recovery_values(&draft);
            self.clear_pending_settings_recovery(candidate, confirmed, cx);
        } else if let Some((candidate, baseline, _)) = self.failed_settings_save.take() {
            self.clear_pending_settings_recovery(
                serde_json::to_value(candidate).expect("AppSettings is JSON serializable"),
                serde_json::to_value(baseline).expect("AppSettings is JSON serializable"),
                cx,
            );
        }
        self.settings.settings_save_error = None;
    }

    pub(super) fn retry_failed_settings_save(&mut self) -> Result<(), WorkbenchError> {
        if self.settings_save_pending() {
            return Err(WorkbenchError::SettingsUnavailable(
                "A settings save is still pending.".into(),
            ));
        }
        if let Some(draft) = self.settings.failed_project_settings_save.as_ref() {
            if !self.project_settings_draft_matches_current_target(draft) {
                return Err(WorkbenchError::SettingsUnavailable(
                    "Select the project that owns this draft before retrying it.".into(),
                ));
            }
            if !self.settings_scope_is_editable(SettingsScope::Project)
                || !self.shared_mutation_allowed()
            {
                return Err(WorkbenchError::SettingsUnavailable(
                    "Host control is required to retry this project settings draft.".into(),
                ));
            }
            self.settings.pending_project_settings_save = Some(draft.clone());
            self.settings.failed_project_settings_save = None;
            self.settings.settings_save_error = None;
            return Ok(());
        }
        let Some((candidate, baseline, bars_only)) = self.failed_settings_save.as_ref() else {
            return Ok(());
        };
        if baseline != &self.settings.confirmed_settings {
            return Err(WorkbenchError::SettingsUnavailable(
                "Settings changed since this draft was created. The draft is retained; review the current values before discarding it and applying your changes again.".into(),
            ));
        }
        if crate::config::scope::host_settings_changed(candidate, baseline)
            && !self.shared_mutation_allowed()
        {
            return Err(WorkbenchError::SettingsUnavailable(
                "Host control is required to retry this settings draft.".into(),
            ));
        }
        self.settings.pending_settings_save = Some((candidate.clone(), *bars_only));
        self.failed_settings_save = None;
        self.settings.settings_save_error = None;
        Ok(())
    }

    pub(in super::super) fn restore_recovered_settings_draft(&mut self) {
        if self.has_failed_settings_save() || self.settings_save_pending() {
            return;
        }
        let Some((candidate, baseline)) = self.take_recoverable_settings_recovery() else {
            return;
        };
        self.stage_recovered_settings_draft(candidate, baseline);
    }

    fn stage_recovered_settings_draft(
        &mut self,
        candidate: serde_json::Value,
        baseline: serde_json::Value,
    ) {
        if candidate.get("kind").is_some() || baseline.get("kind").is_some() {
            match (
                serde_json::from_value::<ProjectSettingsRecoveryCandidate>(candidate),
                serde_json::from_value::<ProjectSettingsRecoveryConfirmed>(baseline),
            ) {
                (Ok(candidate), Ok(baseline))
                    if candidate.project_id == baseline.project_id
                        && candidate.project_path == baseline.project_path
                        && candidate.key == baseline.key
                        && baseline.confirmed.key == candidate.key =>
                {
                    self.settings.failed_project_settings_save = Some(ProjectSettingsSaveDraft {
                        project_id: candidate.project_id,
                        project_path: candidate.project_path,
                        target_generation: 0,
                        key: candidate.key,
                        candidate: candidate.candidate,
                        confirmed: baseline.confirmed,
                    });
                    self.settings.settings_save_error = Some(
                        "A recovered project settings draft is available. Review it before retrying; it has not been applied to the Host.".into(),
                    );
                }
                _ => {
                    self.load_error = Some("Recovered project settings draft is invalid.".into());
                }
            }
            return;
        }
        match (
            serde_json::from_value::<AppSettings>(candidate),
            serde_json::from_value::<AppSettings>(baseline),
        ) {
            (Ok(candidate), Ok(baseline)) => {
                self.failed_settings_save = Some((candidate, baseline, false));
                self.settings.settings_save_error = Some(
                    "A recovered settings draft is available. Review it before retrying; it has not been applied to the Host.".into(),
                );
            }
            (Err(error), _) | (_, Err(error)) => {
                self.load_error = Some(format!("Recovered settings draft is invalid: {error}"));
            }
        }
    }

    /// Pure configuration test backends remain synchronous; live saves run off the UI thread.
    pub(in super::super) fn persist_app_settings(
        &mut self,
        bars_only: bool,
    ) -> Result<bool, WorkbenchError> {
        let candidate = std::mem::replace(
            &mut self.app_settings,
            self.settings.confirmed_settings.clone(),
        );
        if self.settings_save_pending() {
            return Err(WorkbenchError::SettingsUnavailable(
                "A settings save is still pending; wait for its result.".into(),
            ));
        }
        if self.has_failed_settings_save() {
            return Err(WorkbenchError::SettingsUnavailable(
                "A previous settings draft is retained. Retry or discard it before saving another change.".into(),
            ));
        }
        if let Some(runtime) = &self.terminal.host_runtime {
            if !runtime.shared_editing_enabled()
                && crate::config::scope::host_settings_changed(
                    &candidate,
                    &self.settings.confirmed_settings,
                )
            {
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
            crate::config::scope::save_scoped_settings(
                &self.config_paths,
                &candidate,
                &self.settings.confirmed_settings,
                true,
            )
            .map_err(|error| WorkbenchError::SettingsUnavailable(error.to_string()))?;
        }
        if project_editor_defaults_changed(&self.settings.confirmed_settings, &candidate) {
            self.clear_settings_project_target();
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
        let confirmed = self.settings.confirmed_settings.clone();
        let retained = (settings.clone(), confirmed.clone(), bars_only);
        let bars_editor_source = self
            .overlays
            .layout_toml_editor
            .as_ref()
            .filter(|session| {
                bars_only
                    && matches!(session.target(), LayoutEditorTarget::Bars)
                    && validate_bars_editor_source(session.editor().value(), &self.ui_text)
                        .is_ok_and(|bars| bars == settings.bars)
            })
            .map(|session| session.editor().value().to_string());
        let host_changed = crate::config::scope::host_settings_changed(&settings, &confirmed);
        if host_changed {
            self.retain_pending_settings_recovery(
                serde_json::to_value(&settings).expect("AppSettings is JSON serializable"),
                serde_json::to_value(&confirmed).expect("AppSettings is JSON serializable"),
            );
        }
        let runtime = self.terminal.host_runtime.clone();
        let task = cx.background_spawn(async move {
            let theme_store = load_theme_store(&paths).map_err(|error| error.to_string())?;
            let theme = ThemeRuntime::resolve(&settings, &theme_store.store);
            let icons = load_icon_theme(&paths, settings.theme.icon_theme.as_deref())
                .map_err(|error| error.to_string())?;
            if bars_only {
                save_bars(&paths, &settings.bars).map_err(|error| error.to_string())?;
            } else {
                crate::config::scope::save_scoped_settings(
                    &paths,
                    &settings,
                    &confirmed,
                    runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.shared_editing_enabled()),
                )
                .map_err(|error| error.to_string())?;
            }
            Ok::<_, String>((settings, theme, icons))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.settings.settings_save_in_flight = false;
                let same_bars_draft = bars_editor_source.as_deref().is_some_and(|source| {
                    root.overlays
                        .layout_toml_editor
                        .as_ref()
                        .is_some_and(|session| {
                            matches!(session.target(), LayoutEditorTarget::Bars)
                                && session.editor().value() == source
                        })
                });
                match result {
                    Ok((settings, theme, icons)) => {
                        root.settings.settings_save_error = None;
                        root.failed_settings_save = None;
                        if host_changed {
                            root.clear_pending_settings_recovery(
                                serde_json::to_value(&retained.0)
                                    .expect("AppSettings is JSON serializable"),
                                serde_json::to_value(&retained.1)
                                    .expect("AppSettings is JSON serializable"),
                                cx,
                            );
                        }
                        root.apply_confirmed_settings(settings, theme, icons, window, cx);
                        if same_bars_draft {
                            root.cancel_layout_toml_editor();
                        }
                    }
                    Err(error) => {
                        if same_bars_draft {
                            root.set_layout_toml_editor_error("bars", error.clone());
                        }
                        root.load_error = Some(format!("Settings were not saved: {error}"));
                        root.settings.settings_save_error = Some(error);
                        root.failed_settings_save = Some(retained);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in super::super) fn flush_pending_project_settings_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.pending_settings_save.is_some()
            || self.settings.settings_save_in_flight
            || self.settings.project_settings_save_in_flight
        {
            return;
        }
        let Some(draft) = self.settings.pending_project_settings_save.take() else {
            return;
        };
        self.settings.project_settings_save_in_flight = true;
        let paths = self.config_paths.clone();
        let project_path = draft.project_path.clone();
        let key = draft.key;
        let candidate = draft.candidate.clone();
        let confirmed = draft.confirmed.clone();
        let local_test_authority = cfg!(test) || self.local_project_services_for_test;
        let runtime = self.terminal.host_runtime.clone();
        let recovery = project_settings_recovery_values(&draft);
        self.retain_pending_settings_recovery(recovery.0.clone(), recovery.1.clone());
        let task = cx.background_spawn(async move {
            let can_write_host = runtime.as_ref().map_or(local_test_authority, |runtime| {
                runtime.shared_editing_enabled()
            });
            let host_editor_settings = crate::config::settings::load_settings(&paths)
                .map_err(|error| error.to_string())?
                .settings
                .editor;
            crate::config::project_settings::save_effective_project_override_if_matches(
                &paths,
                &project_path,
                &host_editor_settings,
                key,
                candidate,
                &confirmed,
                can_write_host,
            )
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.settings.project_settings_save_in_flight = false;
                match result {
                    Ok(saved) => {
                        root.settings.settings_save_error = None;
                        root.settings.failed_project_settings_save = None;
                        root.clear_pending_settings_recovery(
                            recovery.0.clone(),
                            recovery.1.clone(),
                            cx,
                        );
                        let cache_matches_target = root
                            .settings_project_target()
                            .is_some_and(|(project_id, generation)| {
                                project_id.as_str() == draft.project_id.as_str()
                                    && generation == draft.target_generation
                            });
                        if cache_matches_target {
                            let saved_key = saved.key;
                            if let Some(cached) = root
                                .settings
                                .project_editor_settings
                                .iter_mut()
                                .find(|setting| setting.key == saved_key)
                            {
                                *cached = saved;
                            } else {
                                root.settings.project_editor_settings.push(saved);
                            }
                            match saved_key {
                                crate::config::project_settings::ProjectEditorSettingKey::TabSize => {
                                    root.settings.settings_project_tab_size_input = None;
                                    root.settings.settings_project_tab_size_input_subscription = None;
                                }
                                crate::config::project_settings::ProjectEditorSettingKey::DefaultLanguage => {
                                    root.settings.settings_project_default_language_input = None;
                                    root.settings
                                        .settings_project_default_language_input_subscription = None;
                                }
                                crate::config::project_settings::ProjectEditorSettingKey::AutoDetectLanguage => {}
                            }
                        } else if root.project_settings_draft_matches_current_target(&draft) {
                            root.refresh_settings_project_target(window, cx);
                        }
                    }
                    Err(error) => {
                        root.load_error =
                            Some(format!("Project settings were not saved: {error}"));
                        root.settings.settings_save_error = Some(error);
                        root.settings.failed_project_settings_save = Some(draft);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_confirmed_settings(
        &mut self,
        settings: AppSettings,
        theme: ThemeRuntime,
        icons: IconTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if project_editor_defaults_changed(&self.settings.confirmed_settings, &settings) {
            self.clear_settings_project_target();
        }
        self.settings.confirmed_settings = settings.clone();
        self.app_settings = settings;
        self.appearance.replace(theme);
        self.icon_theme = icons;
        self.ui_text = ui_text_for_language(
            self.onboarding_language()
                .unwrap_or(self.app_settings.general.language),
        );
        self.system_notifications_enabled = self.app_settings.notifications.system;
        self.vim.set_support(self.app_settings.vim.mode);
        self.sync_editor_vim_modes(window, cx);
        if self.app_settings.vim.mode != VimModeSetting::Global {
            for pane in self.terminal.terminal_panes.values() {
                pane.update(cx, |pane, cx| pane.set_terminal_vi_mode(false, cx));
            }
        }
        if self.app_settings.editor.autosave == EditorAutosave::Off {
            self.project
                .project_editor_runtime
                .cancel_all_autosave_tasks();
        }
        self.sync_terminal_environment();
        self.sync_input_owner_state();
        self.sync_vim_controller(window, cx);
        self.settings.keybinding_rows_cache = None;
        window.set_background_appearance(crate::ui::app::window_background_appearance(
            self.app_settings.window.effect,
        ));
        self.apply_appearance_change(window, cx);
    }

    /// Read-only refresh shares Device preferences across windows and observes Host changes.
    /// A snapshot read may never overwrite a save that completed while the read was in flight.
    pub(in super::super) fn ensure_settings_sync(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_sync_task.is_some() || self.terminal.host_runtime.is_none() {
            return;
        }
        self.settings_sync_task = Some(cx.spawn_in(window, async move |this, cx| {
            let mut last_error = None;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(snapshot) = this.update(cx, |root, _| {
                    (!root.settings_save_pending()).then(|| {
                        (
                            root.config_paths.clone(),
                            root.settings.confirmed_settings.clone(),
                            root.terminal.host_runtime.clone(),
                        )
                    })
                }) else {
                    break;
                };
                let Some((paths, confirmed, runtime)) = snapshot else {
                    continue;
                };
                let baseline = confirmed.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let settings = if runtime.as_ref().is_some_and(|runtime| {
                            runtime.is_remote()
                                && !matches!(
                                    runtime.state(),
                                    yttt_client_core::ConnectionState::Ready { .. }
                                )
                        }) {
                            let mut settings = confirmed.clone();
                            crate::config::scope::merge_device_preferences(&paths, &mut settings)
                                .map_err(|error| error.to_string())?;
                            settings
                        } else {
                            crate::config::settings::load_settings(&paths)
                                .map_err(|error| error.to_string())?
                                .settings
                        };
                        if settings == confirmed {
                            return Ok(None);
                        }
                        let theme_store =
                            load_theme_store(&paths).map_err(|error| error.to_string())?;
                        let theme = ThemeRuntime::resolve(&settings, &theme_store.store);
                        let icons = load_icon_theme(&paths, settings.theme.icon_theme.as_deref())
                            .map_err(|error| error.to_string())?;
                        Ok::<_, String>(Some((settings, theme, icons)))
                    })
                    .await;
                match result {
                    Ok(incoming) => {
                        last_error = None;
                        if let Some((settings, theme, icons)) = incoming {
                            let _ = this.update_in(cx, |root, window, cx| {
                                if !root.settings_save_pending()
                                    && root.settings.confirmed_settings == baseline
                                    && root.app_settings == baseline
                                {
                                    root.apply_confirmed_settings(
                                        settings, theme, icons, window, cx,
                                    );
                                    cx.notify();
                                }
                            });
                        }
                    }
                    Err(error) => {
                        if last_error.as_ref() != Some(&error) {
                            let _ = this.update(cx, |root, cx| {
                                root.load_error =
                                    Some(format!("Settings could not be refreshed: {error}"));
                                cx.notify();
                            });
                            last_error = Some(error);
                        }
                    }
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovered_settings_draft_cannot_overwrite_a_newer_confirmed_value() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let mut root = WorkbenchView::with_config_paths(paths.clone());
        let baseline = root.settings.confirmed_settings.clone();
        let mut candidate = baseline.clone();
        candidate.editor.tab_size = 8;
        root.failed_settings_save = Some((candidate, baseline, false));

        root.settings.confirmed_settings.editor.tab_size = 2;
        root.app_settings.editor.tab_size = 2;
        assert!(root.retry_failed_settings_save().is_err());
        assert!(root.has_failed_settings_save());
        assert!(!root.settings_save_pending());
        assert_eq!(root.app_settings.editor.tab_size, 2);
        assert!(!paths.settings_file().exists());
    }

    #[test]
    fn recovered_project_settings_draft_is_staged_without_replaying() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let draft = ProjectSettingsSaveDraft {
            project_id: "project-a".to_string(),
            project_path: temp.path().join("project-a"),
            target_generation: 17,
            key: crate::config::project_settings::ProjectEditorSettingKey::TabSize,
            candidate: Some(crate::config::project_settings::ProjectEditorSettingValue::TabSize(2)),
            confirmed: crate::config::project_settings::EffectiveProjectEditorSetting {
                key: crate::config::project_settings::ProjectEditorSettingKey::TabSize,
                value: crate::config::project_settings::ProjectEditorSettingValue::TabSize(4),
                source: crate::config::project_settings::ProjectSettingSource::Host,
            },
        };
        let (candidate, baseline) = project_settings_recovery_values(&draft);
        let mut root = WorkbenchView::with_config_paths(paths);

        root.stage_recovered_settings_draft(candidate, baseline);

        let recovered = root
            .settings
            .failed_project_settings_save
            .as_ref()
            .expect("recovered project draft should be retained");
        assert_eq!(recovered.project_id, draft.project_id);
        assert_eq!(recovered.project_path, draft.project_path);
        assert_eq!(recovered.key, draft.key);
        assert_eq!(recovered.candidate, draft.candidate);
        assert_eq!(recovered.confirmed, draft.confirmed);
        assert!(root.has_failed_settings_save());
        assert!(root.settings.pending_project_settings_save.is_none());
        assert!(!root.settings.project_settings_save_in_flight);
    }
}
