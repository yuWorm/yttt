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

#[cfg(test)]
mod tests {
    use super::{StartupMode, startup_mode_from_fixture};

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
}
