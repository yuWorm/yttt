use std::time::SystemTime;

use super::*;
use crate::runtime::update::{
    APP_VERSION, UpdateCheck, fetch_update, load_update_cache, save_update_cache,
};
use crate::ui::workbench::state::update::UpdateStatus;

const STARTUP_UPDATE_CHECK_DELAY: Duration = Duration::from_secs(5);
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

impl WorkbenchView {
    pub fn current_app_version(&self) -> &'static str {
        APP_VERSION
    }

    pub fn auto_check_updates_enabled(&self) -> bool {
        self.app_settings.general.auto_check_updates
    }

    pub fn set_auto_check_updates_enabled(&mut self, enabled: bool) -> Result<(), WorkbenchError> {
        self.app_settings.general.auto_check_updates = enabled;
        save_settings(&self.config_paths, &self.app_settings)?;
        Ok(())
    }

    pub fn update_status(&self) -> &UpdateStatus {
        &self.update.status
    }

    pub fn start_update_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.auto_check_updates_enabled()
            || matches!(self.update.status, UpdateStatus::Checking)
        {
            return;
        }
        let cache = load_update_cache(&self.config_paths.update_state_file()).unwrap_or_default();
        if cache.is_due(SystemTime::now()) {
            self.spawn_update_check(true, window, cx);
        }
    }

    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.spawn_update_check(false, window, cx);
    }

    pub fn available_update_url(&self) -> Option<&str> {
        let UpdateStatus::Available(update) = &self.update.status else {
            return None;
        };
        Some(
            update
                .asset
                .as_ref()
                .map(|asset| asset.url.as_str())
                .unwrap_or(update.release_url.as_str()),
        )
    }

    fn spawn_update_check(&mut self, automatic: bool, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.update.status, UpdateStatus::Checking) {
            return;
        }
        self.update.status = UpdateStatus::Checking;
        let http_client = cx.http_client();
        let cache_path = self.config_paths.update_state_file();
        cx.spawn_in(window, async move |this, cx| {
            if automatic {
                cx.background_executor()
                    .timer(STARTUP_UPDATE_CHECK_DELAY)
                    .await;
                let should_run = this
                    .update_in(cx, |root, _window, cx| {
                        let cache = load_update_cache(&root.config_paths.update_state_file())
                            .unwrap_or_default();
                        let should_run =
                            root.auto_check_updates_enabled() && cache.is_due(SystemTime::now());
                        if !should_run {
                            root.update.status = UpdateStatus::Idle;
                            cx.notify();
                        }
                        should_run
                    })
                    .unwrap_or(false);
                if !should_run {
                    return;
                }
            }
            let timeout = async {
                cx.background_executor().timer(UPDATE_CHECK_TIMEOUT).await;
                anyhow::bail!("update check timed out after 30 seconds")
            };
            let result = futures_lite::future::race(fetch_update(http_client), timeout).await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.finish_update_check(result, automatic, &cache_path, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_update_check(
        &mut self,
        result: anyhow::Result<UpdateCheck>,
        automatic: bool,
        cache_path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(UpdateCheck::UpToDate) => {
                let mut cache = load_update_cache(cache_path).unwrap_or_default();
                cache.record_checked(SystemTime::now());
                let _ = save_update_cache(cache_path, &cache);
                self.update.status = UpdateStatus::UpToDate;
                if !automatic {
                    self.push_update_notification(
                        ToastItem {
                            title: self.ui_text.get(UiTextKey::SettingsUpToDate).to_string(),
                            context: format!("v{APP_VERSION}"),
                            tone: ToastTone::Success,
                        },
                        None,
                        window,
                        cx,
                    );
                }
            }
            Ok(UpdateCheck::Available(update)) => {
                let mut cache = load_update_cache(cache_path).unwrap_or_default();
                cache.record_checked(SystemTime::now());
                let should_notify = !automatic || cache.should_notify(&update.version);
                if should_notify {
                    cache.record_notified(&update.version);
                }
                let _ = save_update_cache(cache_path, &cache);
                self.update.status = UpdateStatus::Available(update.clone());
                if should_notify {
                    let url = update
                        .asset
                        .as_ref()
                        .map(|asset| asset.url.clone())
                        .unwrap_or_else(|| update.release_url.clone());
                    self.push_update_notification(
                        ToastItem {
                            title: format!(
                                "{} v{}",
                                self.ui_text.get(UiTextKey::SettingsUpdateAvailable),
                                update.version
                            ),
                            context: self
                                .ui_text
                                .get(UiTextKey::SettingsUpdateAvailableDescription)
                                .to_string(),
                            tone: ToastTone::Success,
                        },
                        Some((
                            self.ui_text
                                .get(UiTextKey::SettingsDownloadUpdate)
                                .to_string(),
                            url,
                        )),
                        window,
                        cx,
                    );
                }
            }
            Err(error) => {
                let mut cache = load_update_cache(cache_path).unwrap_or_default();
                cache.record_checked(SystemTime::now());
                let _ = save_update_cache(cache_path, &cache);
                let message = format!("{error:#}");
                self.update.status = UpdateStatus::Failed(message.clone());
                if !automatic {
                    self.push_update_notification(
                        ToastItem {
                            title: self
                                .ui_text
                                .get(UiTextKey::SettingsUpdateCheckFailed)
                                .to_string(),
                            context: message,
                            tone: ToastTone::Error,
                        },
                        None,
                        window,
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    fn push_update_notification(
        &self,
        item: ToastItem,
        action: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        let notification = if let Some((label, url)) = action {
            workbench_agent_notification(item, label, theme, ui_style)
                .on_click(move |_, _window, cx| cx.open_url(&url))
        } else {
            workbench_status_notification(item, theme, ui_style)
        };
        window.push_notification(notification, cx);
    }
}
