use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

pub const FORCE_ONBOARDING_ENV: &str = "YTTT_FORCE_ONBOARDING";
pub const OPEN_PROJECT_ENV: &str = "YTTT_OPEN_PROJECT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupMode {
    Normal,
    DevFixture,
    AgentExitFixture,
}

pub fn startup_mode_from_fixture(value: Option<&str>) -> StartupMode {
    match value {
        Some("1") => StartupMode::DevFixture,
        Some("agent-exit") => StartupMode::AgentExitFixture,
        _ => StartupMode::Normal,
    }
}

pub fn force_onboarding_from_env(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        ["1", "true", "yes", "on"]
            .iter()
            .any(|enabled| value.eq_ignore_ascii_case(enabled))
    })
}

pub fn startup_project_paths() -> Vec<PathBuf> {
    startup_project_paths_from(std::env::args_os(), std::env::var_os(OPEN_PROJECT_ENV))
}

fn startup_project_paths_from(
    args: impl IntoIterator<Item = OsString>,
    env_project: Option<OsString>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut args = args.into_iter().skip(1);
    while let Some(argument) = args.next() {
        if argument == OsStr::new("--project") || argument == OsStr::new("-p") {
            if let Some(path) = args.next().filter(|path| !path.is_empty()) {
                paths.push(PathBuf::from(path));
            }
            continue;
        }
        if argument == OsStr::new("--") {
            paths.extend(args.filter(|path| !path.is_empty()).map(PathBuf::from));
            break;
        }
        if let Some(argument) = argument.to_str() {
            if let Some(path) = argument.strip_prefix("--project=") {
                if !path.is_empty() {
                    paths.push(PathBuf::from(path));
                }
                continue;
            }
            if argument.starts_with('-') {
                continue;
            }
        }
        if !argument.is_empty() {
            paths.push(PathBuf::from(argument));
        }
    }

    if paths.is_empty()
        && let Some(path) = env_project.filter(|path| !path.is_empty())
    {
        paths.push(PathBuf::from(path));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::{
        OsString, PathBuf, StartupMode, force_onboarding_from_env, startup_mode_from_fixture,
        startup_project_paths_from,
    };

    #[test]
    fn startup_mode_uses_dev_fixture_for_one() {
        assert_eq!(
            startup_mode_from_fixture(Some("1")),
            StartupMode::DevFixture
        );
    }

    #[test]
    fn startup_mode_uses_agent_exit_fixture() {
        assert_eq!(
            startup_mode_from_fixture(Some("agent-exit")),
            StartupMode::AgentExitFixture
        );
    }

    #[test]
    fn startup_mode_falls_back_to_normal_for_missing_or_unknown_fixture() {
        assert_eq!(startup_mode_from_fixture(None), StartupMode::Normal);
        assert_eq!(
            startup_mode_from_fixture(Some("unknown")),
            StartupMode::Normal
        );
    }

    #[test]
    fn force_onboarding_accepts_common_enabled_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " On "] {
            assert!(force_onboarding_from_env(Some(value)), "{value}");
        }
    }

    #[test]
    fn force_onboarding_rejects_missing_or_disabled_values() {
        for value in [None, Some(""), Some("0"), Some("false"), Some("off")] {
            assert!(!force_onboarding_from_env(value), "{value:?}");
        }
    }

    #[test]
    fn startup_paths_accept_platform_neutral_cli_forms() {
        assert_eq!(
            startup_project_paths_from(
                [
                    OsString::from("yttt"),
                    OsString::from("--project"),
                    OsString::from("/first"),
                    OsString::from("-p"),
                    OsString::from("/second"),
                    OsString::from("/third"),
                ],
                Some(OsString::from("/env")),
            ),
            vec![
                PathBuf::from("/first"),
                PathBuf::from("/second"),
                PathBuf::from("/third"),
            ]
        );
    }

    #[test]
    fn startup_paths_ignore_macos_process_serial_number() {
        assert_eq!(
            startup_project_paths_from(
                [
                    OsString::from("yttt"),
                    OsString::from("-psn_0_12345"),
                    OsString::from("--"),
                    OsString::from("/project"),
                ],
                None,
            ),
            vec![PathBuf::from("/project")]
        );
    }

    #[test]
    fn startup_paths_fall_back_to_environment() {
        assert_eq!(
            startup_project_paths_from(
                [OsString::from("yttt")],
                Some(OsString::from("/environment-project")),
            ),
            vec![PathBuf::from("/environment-project")]
        );
    }
}
