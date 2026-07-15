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
enum RevealTargetKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RevealCommand {
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
