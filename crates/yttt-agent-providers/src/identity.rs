//! Names the built-in provider behind a command or executable: the one place
//! that decides which programs are Agents.

use std::borrow::Cow;

use crate::{
    CLAUDE_PROVIDER_ID, CODEX_PROVIDER_ID, GROK_PROVIDER_ID, OMP_PROVIDER_ID, OPENCODE_PROVIDER_ID,
    PI_PROVIDER_ID,
};

const WINDOWS_LAUNCHER_SUFFIXES: [&str; 4] = [".exe", ".cmd", ".bat", ".ps1"];

/// The provider a shell-style command line starts, judged by its first word.
pub fn provider_for_command(command: &str) -> Option<&'static str> {
    provider_for_executable(command.split_whitespace().next()?)
}

/// The provider of an executable path or name, ignoring directories, case
/// and Windows launcher suffixes.
pub fn provider_for_executable(executable: &str) -> Option<&'static str> {
    let stem = executable_stem(executable)?;
    match stem.as_ref() {
        "codex" => Some(CODEX_PROVIDER_ID),
        "claude" | "claude-code" => Some(CLAUDE_PROVIDER_ID),
        "grok" | "groky" => Some(GROK_PROVIDER_ID),
        "opencode" | "open-code" => Some(OPENCODE_PROVIDER_ID),
        "pi" => Some(PI_PROVIDER_ID),
        "omp" | "oh-my-pi" => Some(OMP_PROVIDER_ID),
        stem if stem.starts_with("codex-") => Some(CODEX_PROVIDER_ID),
        _ => None,
    }
}

/// The name a command line's program goes by, for commands no provider claims.
pub fn command_program_name(command: &str) -> Option<&str> {
    basename(command.split_whitespace().next()?)
}

fn basename(executable: &str) -> Option<&str> {
    executable
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
}

fn executable_stem(executable: &str) -> Option<Cow<'_, str>> {
    let basename = basename(executable)?;
    let stem = WINDOWS_LAUNCHER_SUFFIXES
        .into_iter()
        .find(|suffix| {
            basename
                .get(basename.len().saturating_sub(suffix.len())..)
                .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
        })
        .map_or(basename, |suffix| {
            &basename[..basename.len() - suffix.len()]
        });
    Some(if stem.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(stem.to_ascii_lowercase())
    } else {
        Cow::Borrowed(stem)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_a_provider_names_the_same_provider() {
        for (command, provider) in [
            ("codex --model o", CODEX_PROVIDER_ID),
            ("codex-aarch64-apple-darwin", CODEX_PROVIDER_ID),
            ("claude-code", CLAUDE_PROVIDER_ID),
            ("/usr/local/bin/groky", GROK_PROVIDER_ID),
            ("open-code", OPENCODE_PROVIDER_ID),
            ("pi", PI_PROVIDER_ID),
            ("oh-my-pi", OMP_PROVIDER_ID),
            (r"C:\tools\OMP.EXE --resume x", OMP_PROVIDER_ID),
            ("Claude.cmd", CLAUDE_PROVIDER_ID),
        ] {
            assert_eq!(provider_for_command(command), Some(provider), "{command}");
        }
    }

    #[test]
    fn other_programs_and_arguments_name_no_provider() {
        for command in ["", "   ", "npm run pi", "pip", "/usr/bin/fish"] {
            assert_eq!(provider_for_command(command), None, "{command}");
        }
        assert_eq!(
            command_program_name("/opt/bin/wrapper --flag"),
            Some("wrapper")
        );
    }
}
