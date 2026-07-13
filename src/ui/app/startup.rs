pub const FORCE_ONBOARDING_ENV: &str = "YTTT_FORCE_ONBOARDING";

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

#[cfg(test)]
mod tests {
    use super::{StartupMode, force_onboarding_from_env, startup_mode_from_fixture};

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
}
