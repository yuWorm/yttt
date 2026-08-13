use yttt_protocol::{HostLifecycleState, HostLifecycleStatus};

const ACTION_QUEUE_CAPACITY: usize = 32;
const OPEN_ID: &str = "yttt.tray.open";
const NEW_WINDOW_ID: &str = "yttt.tray.new-window";
const OPEN_LOGS_ID: &str = "yttt.tray.open-logs";
const START_HOST_ID: &str = "yttt.tray.start-host";
const STOP_HOST_ID: &str = "yttt.tray.stop-host";
const RESTART_HOST_ID: &str = "yttt.tray.restart-host";
const QUIT_DESKTOP_ID: &str = "yttt.tray.quit-desktop";
const QUIT_ALL_ID: &str = "yttt.tray.quit-all";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopTrayAction {
    Open,
    NewWindow,
    OpenLogs,
    StartHost,
    StopHost,
    RestartHost,
    QuitDesktop,
    QuitAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopTrayHostState {
    Unavailable,
    Running,
    Draining,
    ForceStopping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopTrayStatus {
    pub state: DesktopTrayHostState,
    pub terminal_count: u32,
    pub client_count: u32,
    pub project_count: u32,
    pub ssh_connection_count: u32,
    pub job_count: u32,
    pub blocker_count: usize,
    pub detail: Option<String>,
}

impl DesktopTrayStatus {
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            state: DesktopTrayHostState::Unavailable,
            terminal_count: 0,
            client_count: 0,
            project_count: 0,
            ssh_connection_count: 0,
            job_count: 0,
            blocker_count: 0,
            detail: Some(detail.into()),
        }
    }

    pub fn from_lifecycle(status: &HostLifecycleStatus) -> Self {
        Self {
            state: match status.state {
                HostLifecycleState::Running => DesktopTrayHostState::Running,
                HostLifecycleState::Draining => DesktopTrayHostState::Draining,
                HostLifecycleState::ForceStopping => DesktopTrayHostState::ForceStopping,
            },
            terminal_count: status.terminal_count,
            client_count: status.client_count,
            project_count: status.project_count,
            ssh_connection_count: status.ssh_connection_count,
            job_count: status.agent_count,
            blocker_count: status.blockers.len(),
            detail: None,
        }
    }

    pub fn menu_label(&self) -> String {
        if let Some(detail) = &self.detail {
            return format!("Host: {detail}");
        }
        let state = match self.state {
            DesktopTrayHostState::Unavailable => "Unavailable",
            DesktopTrayHostState::Running => "Running",
            DesktopTrayHostState::Draining => "Draining",
            DesktopTrayHostState::ForceStopping => "Stopping",
        };
        format!(
            "Host: {state} · {} terminals · {} clients · {} jobs",
            self.terminal_count, self.client_count, self.job_count
        )
    }

    pub fn host_available(&self) -> bool {
        self.state != DesktopTrayHostState::Unavailable
    }
}

pub trait DesktopTrayAdapter {
    fn is_available(&self) -> bool;
    fn actions(&self) -> flume::Receiver<DesktopTrayAction>;
    fn update(&self, status: &DesktopTrayStatus);
}

pub fn create_desktop_tray() -> Result<Box<dyn DesktopTrayAdapter>, DesktopTrayError> {
    platform::create()
}

fn action_for_id(id: &str) -> Option<DesktopTrayAction> {
    match id {
        OPEN_ID => Some(DesktopTrayAction::Open),
        NEW_WINDOW_ID => Some(DesktopTrayAction::NewWindow),
        OPEN_LOGS_ID => Some(DesktopTrayAction::OpenLogs),
        START_HOST_ID => Some(DesktopTrayAction::StartHost),
        STOP_HOST_ID => Some(DesktopTrayAction::StopHost),
        RESTART_HOST_ID => Some(DesktopTrayAction::RestartHost),
        QUIT_DESKTOP_ID => Some(DesktopTrayAction::QuitDesktop),
        QUIT_ALL_ID => Some(DesktopTrayAction::QuitAll),
        _ => None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DesktopTrayError {
    #[error("failed to initialize desktop tray: {0}")]
    Initialization(String),
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use super::*;
    use tray_icon::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    pub(super) fn create() -> Result<Box<dyn DesktopTrayAdapter>, DesktopTrayError> {
        Ok(Box::new(NativeDesktopTray::new()?))
    }

    struct NativeDesktopTray {
        _tray: TrayIcon,
        status: MenuItem,
        start_host: MenuItem,
        stop_host: MenuItem,
        restart_host: MenuItem,
        actions: flume::Receiver<DesktopTrayAction>,
    }

    impl NativeDesktopTray {
        fn new() -> Result<Self, DesktopTrayError> {
            let status = MenuItem::with_id("yttt.tray.status", "Host: Connecting", false, None);
            let open = MenuItem::with_id(OPEN_ID, "Open yttt", true, None);
            let new_window = MenuItem::with_id(NEW_WINDOW_ID, "New Window", true, None);
            let open_logs = MenuItem::with_id(OPEN_LOGS_ID, "Open Logs", true, None);
            let start_host = MenuItem::with_id(START_HOST_ID, "Start Host", false, None);
            let stop_host = MenuItem::with_id(STOP_HOST_ID, "Stop Host If Idle", true, None);
            let restart_host =
                MenuItem::with_id(RESTART_HOST_ID, "Restart Host If Idle", true, None);
            let quit_desktop = MenuItem::with_id(QUIT_DESKTOP_ID, "Quit Desktop", true, None);
            let quit_all = MenuItem::with_id(QUIT_ALL_ID, "Quit All", true, None);
            let separator_one = PredefinedMenuItem::separator();
            let separator_two = PredefinedMenuItem::separator();
            let separator_three = PredefinedMenuItem::separator();
            let menu = Menu::with_items(&[
                &status,
                &separator_one,
                &open,
                &new_window,
                &open_logs,
                &separator_two,
                &start_host,
                &stop_host,
                &restart_host,
                &separator_three,
                &quit_desktop,
                &quit_all,
            ])
            .map_err(|error| DesktopTrayError::Initialization(error.to_string()))?;
            let icon = tray_icon()?;
            let tray = TrayIconBuilder::new()
                .with_id("yttt.desktop-tray")
                .with_menu(Box::new(menu))
                .with_tooltip("yttt")
                .with_icon(icon)
                .with_icon_as_template(cfg!(target_os = "macos"))
                .build()
                .map_err(|error| DesktopTrayError::Initialization(error.to_string()))?;
            let (actions_tx, actions) = flume::bounded(ACTION_QUEUE_CAPACITY);
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(action) = action_for_id(event.id.as_ref()) {
                    let _ = actions_tx.try_send(action);
                }
            }));
            Ok(Self {
                _tray: tray,
                status,
                start_host,
                stop_host,
                restart_host,
                actions,
            })
        }
    }

    impl DesktopTrayAdapter for NativeDesktopTray {
        fn is_available(&self) -> bool {
            true
        }

        fn actions(&self) -> flume::Receiver<DesktopTrayAction> {
            self.actions.clone()
        }

        fn update(&self, status: &DesktopTrayStatus) {
            let label = status.menu_label();
            self.status.set_text(&label);
            self.start_host.set_enabled(!status.host_available());
            self.stop_host.set_enabled(status.host_available());
            self.restart_host.set_enabled(status.host_available());
            let _ = self._tray.set_tooltip(Some(label));
        }
    }

    impl Drop for NativeDesktopTray {
        fn drop(&mut self) {
            MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        }
    }

    fn tray_icon() -> Result<Icon, DesktopTrayError> {
        const SIZE: u32 = 18;
        let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
        for y in 2_u32..9 {
            for center in [y, SIZE - 1 - y] {
                for x in center.saturating_sub(1)..=(center + 1).min(SIZE - 1) {
                    let offset = ((y * SIZE + x) * 4) as usize;
                    rgba[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        for y in 9_u32..16 {
            for x in 7_u32..=10 {
                let offset = ((y * SIZE + x) * 4) as usize;
                rgba[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        Icon::from_rgba(rgba, SIZE, SIZE)
            .map_err(|error| DesktopTrayError::Initialization(error.to_string()))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) fn create() -> Result<Box<dyn DesktopTrayAdapter>, DesktopTrayError> {
        Ok(Box::new(UnavailableDesktopTray::new()))
    }

    struct UnavailableDesktopTray {
        _actions_tx: flume::Sender<DesktopTrayAction>,
        actions: flume::Receiver<DesktopTrayAction>,
    }

    impl UnavailableDesktopTray {
        fn new() -> Self {
            let (actions_tx, actions) = flume::bounded(ACTION_QUEUE_CAPACITY);
            Self {
                _actions_tx: actions_tx,
                actions,
            }
        }
    }

    impl DesktopTrayAdapter for UnavailableDesktopTray {
        fn is_available(&self) -> bool {
            false
        }

        fn actions(&self) -> flume::Receiver<DesktopTrayAction> {
            self.actions.clone()
        }

        fn update(&self, _status: &DesktopTrayStatus) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_protocol::{
        BuildIdentity, HostBlocker, LIFECYCLE_PROTOCOL_VERSION, RESOURCE_PROTOCOL_VERSION,
    };

    #[test]
    fn native_menu_ids_map_to_bounded_actions() {
        assert_eq!(action_for_id(OPEN_ID), Some(DesktopTrayAction::Open));
        assert_eq!(
            action_for_id(RESTART_HOST_ID),
            Some(DesktopTrayAction::RestartHost)
        );
        assert_eq!(action_for_id("not-yttt"), None);
    }

    #[test]
    fn lifecycle_status_exposes_terminal_client_and_job_counts() {
        let status = HostLifecycleStatus {
            lifecycle_protocol: LIFECYCLE_PROTOCOL_VERSION,
            resource_protocol: RESOURCE_PROTOCOL_VERSION,
            build: BuildIdentity {
                product_version: "0.2.0".to_string(),
                build_fingerprint: "test".to_string(),
                resource_compatibility: "resource-v1".to_string(),
            },
            state: HostLifecycleState::Running,
            terminal_count: 2,
            client_count: 1,
            project_count: 3,
            ssh_connection_count: 1,
            agent_count: 4,
            blockers: vec![HostBlocker::PendingResourceOperations { count: 1 }],
        };

        let tray = DesktopTrayStatus::from_lifecycle(&status);

        assert_eq!(tray.terminal_count, 2);
        assert_eq!(tray.client_count, 1);
        assert_eq!(tray.job_count, 4);
        assert_eq!(tray.blocker_count, 1);
        assert_eq!(
            tray.menu_label(),
            "Host: Running · 2 terminals · 1 clients · 4 jobs"
        );
    }
}
