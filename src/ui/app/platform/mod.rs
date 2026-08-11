use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
};

use crate::config::settings::WindowBackgroundEffect;

pub const APP_ID: &str = "com.yttt.app";

#[cfg(target_os = "macos")]
pub mod macos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DesktopPlatform {
    MacOs,
    Windows,
    Linux,
}

impl DesktopPlatform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionKind {
    Notifications,
    FileSystem,
    DeveloperTools,
    Accessibility,
    ScreenCapture,
}

impl PermissionKind {
    pub const ALL: [Self; 5] = [
        Self::Notifications,
        Self::FileSystem,
        Self::DeveloperTools,
        Self::Accessibility,
        Self::ScreenCapture,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        match self {
            Self::Notifications => 0,
            Self::FileSystem => 1,
            Self::DeveloperTools => 2,
            Self::Accessibility => 3,
            Self::ScreenCapture => 4,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Notifications => "notifications",
            Self::FileSystem => "file-system",
            Self::DeveloperTools => "developer-tools",
            Self::Accessibility => "accessibility",
            Self::ScreenCapture => "screen-capture",
        }
    }

    pub const fn is_optional(self) -> bool {
        matches!(self, Self::Accessibility | Self::ScreenCapture)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionControl {
    SystemSettings,
    ManagedBySystem,
    RequestedWhenNeeded,
    NotRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionStatus {
    Checking,
    Unknown,
    NotDetermined,
    Granted,
    Denied,
    Unavailable,
    ManagedBySystem,
    RequestedWhenNeeded,
    NotRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionAction {
    Request,
    OpenSettings,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermissionActionResult {
    pub status: PermissionStatus,
    pub opened_system_settings: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformPermission {
    pub kind: PermissionKind,
    pub control: PermissionControl,
}

const MACOS_PERMISSIONS: &[PlatformPermission] = &[
    PlatformPermission {
        kind: PermissionKind::Notifications,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::FileSystem,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::DeveloperTools,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::Accessibility,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::ScreenCapture,
        control: PermissionControl::SystemSettings,
    },
];

const WINDOWS_PERMISSIONS: &[PlatformPermission] = &[
    PlatformPermission {
        kind: PermissionKind::Notifications,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::FileSystem,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::DeveloperTools,
        control: PermissionControl::SystemSettings,
    },
    PlatformPermission {
        kind: PermissionKind::Accessibility,
        control: PermissionControl::NotRequired,
    },
    PlatformPermission {
        kind: PermissionKind::ScreenCapture,
        control: PermissionControl::SystemSettings,
    },
];

const LINUX_PERMISSIONS: &[PlatformPermission] = &[
    PlatformPermission {
        kind: PermissionKind::Notifications,
        control: PermissionControl::ManagedBySystem,
    },
    PlatformPermission {
        kind: PermissionKind::FileSystem,
        control: PermissionControl::NotRequired,
    },
    PlatformPermission {
        kind: PermissionKind::DeveloperTools,
        control: PermissionControl::NotRequired,
    },
    PlatformPermission {
        kind: PermissionKind::Accessibility,
        control: PermissionControl::RequestedWhenNeeded,
    },
    PlatformPermission {
        kind: PermissionKind::ScreenCapture,
        control: PermissionControl::RequestedWhenNeeded,
    },
];

pub fn platform_permissions() -> &'static [PlatformPermission] {
    permissions_for(DesktopPlatform::current())
}

fn permissions_for(platform: DesktopPlatform) -> &'static [PlatformPermission] {
    match platform {
        DesktopPlatform::MacOs => MACOS_PERMISSIONS,
        DesktopPlatform::Windows => WINDOWS_PERMISSIONS,
        DesktopPlatform::Linux => LINUX_PERMISSIONS,
    }
}

pub fn initial_permission_status(kind: PermissionKind) -> PermissionStatus {
    initial_permission_status_for(DesktopPlatform::current(), kind)
}

fn initial_permission_status_for(
    platform: DesktopPlatform,
    kind: PermissionKind,
) -> PermissionStatus {
    let control = permissions_for(platform)
        .iter()
        .find(|permission| permission.kind == kind)
        .map(|permission| permission.control)
        .unwrap_or(PermissionControl::NotRequired);
    match control {
        PermissionControl::SystemSettings => PermissionStatus::Unknown,
        PermissionControl::ManagedBySystem => PermissionStatus::ManagedBySystem,
        PermissionControl::RequestedWhenNeeded => PermissionStatus::RequestedWhenNeeded,
        PermissionControl::NotRequired => PermissionStatus::NotRequired,
    }
}

pub fn detect_permission_status(kind: PermissionKind) -> io::Result<PermissionStatus> {
    #[cfg(target_os = "macos")]
    {
        macos::detect_permission_status(kind)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let status = initial_permission_status(kind);
        Ok(if status == PermissionStatus::Unknown {
            PermissionStatus::Unavailable
        } else {
            status
        })
    }
}

pub fn permission_action(
    kind: PermissionKind,
    status: PermissionStatus,
    request_attempted: bool,
) -> PermissionAction {
    permission_action_for(DesktopPlatform::current(), kind, status, request_attempted)
}

fn permission_action_for(
    platform: DesktopPlatform,
    kind: PermissionKind,
    status: PermissionStatus,
    request_attempted: bool,
) -> PermissionAction {
    if status == PermissionStatus::Checking {
        return PermissionAction::None;
    }

    if platform == DesktopPlatform::MacOs {
        if kind == PermissionKind::Notifications && status == PermissionStatus::NotDetermined {
            return PermissionAction::Request;
        }
        if matches!(
            kind,
            PermissionKind::Accessibility | PermissionKind::ScreenCapture
        ) && status == PermissionStatus::Denied
            && !request_attempted
        {
            return PermissionAction::Request;
        }
    }

    let opens_settings = permissions_for(platform)
        .iter()
        .find(|permission| permission.kind == kind)
        .is_some_and(|permission| permission.control == PermissionControl::SystemSettings);
    if opens_settings {
        PermissionAction::OpenSettings
    } else {
        PermissionAction::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RevealTargetKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RevealCommand {
    program: &'static str,
    args: Vec<OsString>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SystemSettingsCommand {
    program: &'static str,
    args: Vec<OsString>,
}

pub fn reveal_path(path: &Path) -> io::Result<()> {
    let requested = absolute_path(path)?;
    let target = nearest_existing_path(&requested);
    let kind = if target.is_file() {
        RevealTargetKind::File
    } else {
        RevealTargetKind::Directory
    };
    let command = reveal_command(DesktopPlatform::current(), &target, kind);
    Command::new(command.program).args(command.args).spawn()?;
    Ok(())
}

pub fn open_permission_settings(kind: PermissionKind) -> io::Result<bool> {
    let Some(command) = permission_settings_command(DesktopPlatform::current(), kind) else {
        return Ok(false);
    };
    Command::new(command.program).args(command.args).spawn()?;
    Ok(true)
}

pub fn perform_permission_action(
    kind: PermissionKind,
    status: PermissionStatus,
    request_attempted: bool,
) -> io::Result<PermissionActionResult> {
    match permission_action(kind, status, request_attempted) {
        PermissionAction::Request => {
            #[cfg(target_os = "macos")]
            {
                Ok(PermissionActionResult {
                    status: macos::request_native_permission(kind)?,
                    opened_system_settings: false,
                })
            }
            #[cfg(not(target_os = "macos"))]
            {
                Ok(PermissionActionResult {
                    status,
                    opened_system_settings: false,
                })
            }
        }
        PermissionAction::OpenSettings => Ok(PermissionActionResult {
            status,
            opened_system_settings: open_permission_settings(kind)?,
        }),
        PermissionAction::None => Ok(PermissionActionResult {
            status,
            opened_system_settings: false,
        }),
    }
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn nearest_existing_path(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|candidate| candidate.exists())
        .unwrap_or(path)
        .to_path_buf()
}

fn reveal_command(
    platform: DesktopPlatform,
    target: &Path,
    kind: RevealTargetKind,
) -> RevealCommand {
    match platform {
        DesktopPlatform::MacOs => {
            let mut args = Vec::with_capacity(2);
            if kind == RevealTargetKind::File {
                args.push(OsString::from("-R"));
            }
            args.push(target.as_os_str().to_os_string());
            RevealCommand {
                program: "open",
                args,
            }
        }
        DesktopPlatform::Windows => {
            let args = if kind == RevealTargetKind::File {
                let mut select = OsString::from("/select,");
                select.push(target.as_os_str());
                vec![select]
            } else {
                vec![target.as_os_str().to_os_string()]
            };
            RevealCommand {
                program: "explorer.exe",
                args,
            }
        }
        DesktopPlatform::Linux => {
            let directory = if kind == RevealTargetKind::File {
                target.parent().unwrap_or(target)
            } else {
                target
            };
            RevealCommand {
                program: "xdg-open",
                args: vec![directory.as_os_str().to_os_string()],
            }
        }
    }
}

fn permission_settings_command(
    platform: DesktopPlatform,
    kind: PermissionKind,
) -> Option<SystemSettingsCommand> {
    let (program, target) = match (platform, kind) {
        (DesktopPlatform::MacOs, PermissionKind::Notifications) => (
            "open",
            OsString::from(format!(
                "x-apple.systempreferences:com.apple.Notifications-Settings.extension?bundleIdentifier={APP_ID}"
            )),
        ),
        (DesktopPlatform::MacOs, PermissionKind::FileSystem) => (
            "open",
            OsString::from(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
            ),
        ),
        (DesktopPlatform::MacOs, PermissionKind::DeveloperTools) => (
            "open",
            OsString::from(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_DeveloperTools",
            ),
        ),
        (DesktopPlatform::MacOs, PermissionKind::Accessibility) => (
            "open",
            OsString::from(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            ),
        ),
        (DesktopPlatform::MacOs, PermissionKind::ScreenCapture) => (
            "open",
            OsString::from(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
            ),
        ),
        (DesktopPlatform::Windows, PermissionKind::Notifications) => {
            ("explorer.exe", OsString::from("ms-settings:notifications"))
        }
        (DesktopPlatform::Windows, PermissionKind::FileSystem) => (
            "explorer.exe",
            OsString::from("ms-settings:privacy-broadfilesystemaccess"),
        ),
        (DesktopPlatform::Windows, PermissionKind::DeveloperTools) => {
            ("explorer.exe", OsString::from("ms-settings:developers"))
        }
        (DesktopPlatform::Windows, PermissionKind::ScreenCapture) => (
            "explorer.exe",
            OsString::from("ms-settings:privacy-graphicscaptureprogrammatic"),
        ),
        _ => return None,
    };
    Some(SystemSettingsCommand {
        program,
        args: vec![target],
    })
}

pub fn resolved_window_background_effect(
    requested: WindowBackgroundEffect,
) -> WindowBackgroundEffect {
    resolve_window_background_effect(
        requested,
        DesktopPlatform::current(),
        current_platform_supports_blur(),
    )
}

fn resolve_window_background_effect(
    requested: WindowBackgroundEffect,
    platform: DesktopPlatform,
    blur_supported: bool,
) -> WindowBackgroundEffect {
    if requested == WindowBackgroundEffect::Blurred
        && platform == DesktopPlatform::Linux
        && !blur_supported
    {
        WindowBackgroundEffect::None
    } else {
        requested
    }
}

fn current_platform_supports_blur() -> bool {
    match DesktopPlatform::current() {
        DesktopPlatform::MacOs | DesktopPlatform::Windows => true,
        DesktopPlatform::Linux => linux_blur_supported_from_parts(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
            std::env::var("KDE_FULL_SESSION").ok().as_deref(),
        ),
    }
}

fn linux_blur_supported_from_parts(
    wayland_display: Option<&str>,
    session_type: Option<&str>,
    current_desktop: Option<&str>,
    kde_full_session: Option<&str>,
) -> bool {
    let uses_wayland = wayland_display.is_some_and(|value| !value.trim().is_empty())
        || session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"));
    let uses_kde = current_desktop.is_some_and(|value| {
        value.split([':', ';']).any(|desktop| {
            matches!(
                desktop.trim().to_ascii_lowercase().as_str(),
                "kde" | "plasma"
            )
        })
    }) || kde_full_session.is_some_and(|value| {
        let value = value.trim();
        !value.is_empty() && !value.eq_ignore_ascii_case("false")
    });

    uses_wayland && uses_kde
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_reveals_files_and_opens_directories() {
        assert_eq!(
            reveal_command(
                DesktopPlatform::MacOs,
                Path::new("/tmp/project/layout.toml"),
                RevealTargetKind::File,
            ),
            RevealCommand {
                program: "open",
                args: vec![
                    OsString::from("-R"),
                    OsString::from("/tmp/project/layout.toml")
                ],
            }
        );
        assert_eq!(
            reveal_command(
                DesktopPlatform::MacOs,
                Path::new("/tmp/project"),
                RevealTargetKind::Directory,
            ),
            RevealCommand {
                program: "open",
                args: vec![OsString::from("/tmp/project")],
            }
        );
    }

    #[test]
    fn windows_uses_explorer_select_for_files() {
        assert_eq!(
            reveal_command(
                DesktopPlatform::Windows,
                Path::new(r"C:\project\layout.toml"),
                RevealTargetKind::File,
            ),
            RevealCommand {
                program: "explorer.exe",
                args: vec![OsString::from(r"/select,C:\project\layout.toml")],
            }
        );
    }

    #[test]
    fn linux_opens_the_containing_directory() {
        assert_eq!(
            reveal_command(
                DesktopPlatform::Linux,
                Path::new("/tmp/project/layout.toml"),
                RevealTargetKind::File,
            ),
            RevealCommand {
                program: "xdg-open",
                args: vec![OsString::from("/tmp/project")],
            }
        );
    }

    #[test]
    fn permission_catalog_reflects_each_desktop_security_model() {
        assert!(
            permissions_for(DesktopPlatform::MacOs)
                .iter()
                .all(|permission| permission.control == PermissionControl::SystemSettings)
        );
        assert_eq!(
            permissions_for(DesktopPlatform::Windows)
                .iter()
                .find(|permission| permission.kind == PermissionKind::Accessibility)
                .map(|permission| permission.control),
            Some(PermissionControl::NotRequired)
        );
        assert_eq!(
            permissions_for(DesktopPlatform::Linux)
                .iter()
                .find(|permission| permission.kind == PermissionKind::ScreenCapture)
                .map(|permission| permission.control),
            Some(PermissionControl::RequestedWhenNeeded)
        );
    }

    #[test]
    fn permission_settings_commands_target_native_settings_pages() {
        assert_eq!(
            permission_settings_command(DesktopPlatform::MacOs, PermissionKind::Notifications),
            Some(SystemSettingsCommand {
                program: "open",
                args: vec![OsString::from(
                    "x-apple.systempreferences:com.apple.Notifications-Settings.extension?bundleIdentifier=com.yttt.app"
                )],
            })
        );
        assert_eq!(
            permission_settings_command(DesktopPlatform::Windows, PermissionKind::FileSystem),
            Some(SystemSettingsCommand {
                program: "explorer.exe",
                args: vec![OsString::from("ms-settings:privacy-broadfilesystemaccess")],
            })
        );
        assert_eq!(
            permission_settings_command(DesktopPlatform::Linux, PermissionKind::FileSystem),
            None
        );
    }

    #[test]
    fn macos_requests_once_before_falling_back_to_system_settings() {
        assert_eq!(
            permission_action_for(
                DesktopPlatform::MacOs,
                PermissionKind::Notifications,
                PermissionStatus::NotDetermined,
                false,
            ),
            PermissionAction::Request
        );
        assert_eq!(
            permission_action_for(
                DesktopPlatform::MacOs,
                PermissionKind::ScreenCapture,
                PermissionStatus::Denied,
                false,
            ),
            PermissionAction::Request
        );
        assert_eq!(
            permission_action_for(
                DesktopPlatform::MacOs,
                PermissionKind::ScreenCapture,
                PermissionStatus::Denied,
                true,
            ),
            PermissionAction::OpenSettings
        );
    }

    #[test]
    fn non_requestable_permissions_explain_their_platform_status() {
        assert_eq!(
            initial_permission_status_for(DesktopPlatform::Linux, PermissionKind::ScreenCapture),
            PermissionStatus::RequestedWhenNeeded
        );
        assert_eq!(
            initial_permission_status_for(DesktopPlatform::Windows, PermissionKind::Accessibility),
            PermissionStatus::NotRequired
        );
        assert_eq!(
            permission_action_for(
                DesktopPlatform::Linux,
                PermissionKind::FileSystem,
                PermissionStatus::NotRequired,
                false,
            ),
            PermissionAction::None
        );
    }

    #[test]
    fn linux_blur_requires_kde_wayland() {
        assert!(linux_blur_supported_from_parts(
            Some("wayland-0"),
            Some("wayland"),
            Some("KDE"),
            None,
        ));
        assert!(!linux_blur_supported_from_parts(
            None,
            Some("x11"),
            Some("KDE"),
            Some("true"),
        ));
        assert!(!linux_blur_supported_from_parts(
            Some("wayland-0"),
            Some("wayland"),
            Some("GNOME"),
            None,
        ));
    }

    #[test]
    fn unsupported_linux_blur_falls_back_to_opaque() {
        assert_eq!(
            resolve_window_background_effect(
                WindowBackgroundEffect::Blurred,
                DesktopPlatform::Linux,
                false,
            ),
            WindowBackgroundEffect::None
        );
        assert_eq!(
            resolve_window_background_effect(
                WindowBackgroundEffect::Transparent,
                DesktopPlatform::Linux,
                false,
            ),
            WindowBackgroundEffect::Transparent
        );
        assert_eq!(
            resolve_window_background_effect(
                WindowBackgroundEffect::Blurred,
                DesktopPlatform::Windows,
                true,
            ),
            WindowBackgroundEffect::Blurred
        );
    }
}
