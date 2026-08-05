use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use yttt_agent_providers::{OMP_EXTENSION_SOURCE, OPENCODE_PLUGIN_SOURCE, PI_EXTENSION_SOURCE};

use crate::config::{atomic_write, paths::AppConfigPaths};

const MANAGED_HOOK_FILE_NAME: &str = "yttt-agent-hook";
const MANAGED_MARKER: &str = "yttt-agent-hook";
const HOOK_TIMEOUT_SECONDS: u64 = 10;
const CLAUDE_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "StopFailure",
];
const CODEX_EVENTS: &[(&str, &str)] = &[
    ("SessionStart", "session_start"),
    ("UserPromptSubmit", "user_prompt_submit"),
    ("PreToolUse", "pre_tool_use"),
    ("PermissionRequest", "permission_request"),
    ("PostToolUse", "post_tool_use"),
    ("SubagentStart", "subagent_start"),
    ("SubagentStop", "subagent_stop"),
    ("Stop", "stop"),
];

pub fn install_managed_hooks(config_paths: &AppConfigPaths) -> io::Result<()> {
    if cfg!(test) || config_paths.config_dir() != AppConfigPaths::for_app().config_dir() {
        return Ok(());
    }
    let home = home_dir()?;
    install_managed_hooks_at(config_paths, &home)
}

fn install_managed_hooks_at(config_paths: &AppConfigPaths, home: &Path) -> io::Result<()> {
    let hook_dir = config_paths.config_dir().join("agent-hooks");
    fs::create_dir_all(&hook_dir)?;
    restrict_directory(&hook_dir)?;
    let script_path = hook_dir.join(managed_hook_file_name());
    write_executable(&script_path, managed_hook_source().as_bytes())?;

    let mut errors = Vec::new();
    collect_error(&mut errors, install_claude(home, &script_path));
    collect_error(&mut errors, install_codex(home, &script_path));
    collect_error(&mut errors, install_opencode(home));
    collect_error(&mut errors, install_pi_and_omp(home));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

fn collect_error(errors: &mut Vec<String>, result: io::Result<()>) {
    if let Err(error) = result {
        errors.push(error.to_string());
    }
}

fn install_claude(home: &Path, script_path: &Path) -> io::Result<()> {
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let settings_path = config_dir.join("settings.json");
    let mut settings = read_json_object(&settings_path)?;
    let command = managed_command(script_path, "claude");
    let hooks = object_entry(&mut settings, "hooks")?;
    for event in CLAUDE_EVENTS {
        upsert_json_hook(hooks, event, &command)?;
    }
    write_json(&settings_path, Value::Object(settings))
        .map_err(|error| with_context("Claude hook install", error))
}

fn install_codex(home: &Path, script_path: &Path) -> io::Result<()> {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let hooks_path = codex_home.join("hooks.json");
    let mut settings = read_json_object(&hooks_path)?;
    let command = managed_command(script_path, "codex");
    let hooks = object_entry(&mut settings, "hooks")?;
    let mut trust_entries = Vec::with_capacity(CODEX_EVENTS.len());
    for (event, event_label) in CODEX_EVENTS {
        let (group_index, handler_index) = upsert_json_hook(hooks, event, &command)?;
        trust_entries.push((event_label.to_string(), group_index, handler_index));
    }
    write_json(&hooks_path, Value::Object(settings))
        .map_err(|error| with_context("Codex hook install", error))?;

    let config_path = codex_home.join("config.toml");
    let mut config = match fs::read_to_string(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(with_context("Codex trust read", error)),
    };
    for (event_label, group_index, handler_index) in trust_entries {
        let key = format!(
            "{}:{event_label}:{group_index}:{handler_index}",
            hooks_path.to_string_lossy()
        );
        let hash = codex_hook_hash(&event_label, &command, HOOK_TIMEOUT_SECONDS);
        config = upsert_codex_trust_block(&config, &key, &hash);
    }
    atomic_write_with_parent(&config_path, config.as_bytes())
        .map_err(|error| with_context("Codex trust write", error))
}

fn install_opencode(home: &Path) -> io::Result<()> {
    let config_dir = std::env::var_os("OPENCODE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").map(|root| PathBuf::from(root).join("opencode"))
        })
        .unwrap_or_else(|| home.join(".config").join("opencode"));
    let path = config_dir.join("plugins").join("yttt-agent-status.js");
    atomic_write_with_parent(&path, OPENCODE_PLUGIN_SOURCE.as_bytes())
        .map_err(|error| with_context("OpenCode plugin install", error))
}

fn install_pi_and_omp(home: &Path) -> io::Result<()> {
    let omp_path = home
        .join(".omp")
        .join("agent")
        .join("extensions")
        .join("yttt-agent-status.ts");
    atomic_write_with_parent(&omp_path, OMP_EXTENSION_SOURCE.as_bytes())
        .map_err(|error| with_context("Oh My Pi extension install", error))?;

    let pi_source = PI_EXTENSION_SOURCE;
    let pi_path = home
        .join(".pi")
        .join("agent")
        .join("extensions")
        .join("yttt-agent-status.ts");
    atomic_write_with_parent(&pi_path, pi_source.as_bytes())
        .map_err(|error| with_context("Pi extension install", error))
}

fn read_json_object(path: &Path) -> io::Result<Map<String, Value>> {
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} exceeds the 1 MiB safety limit", path.display()),
                ));
            }
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .as_object()
                .cloned()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{} must contain a JSON object", path.display()),
                    )
                })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Map::new()),
        Err(error) => Err(error),
    }
}

fn object_entry<'a>(
    object: &'a mut Map<String, Value>,
    name: &str,
) -> io::Result<&'a mut Map<String, Value>> {
    let value = object
        .entry(name.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    value.as_object_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{name} must contain a JSON object"),
        )
    })
}

fn upsert_json_hook(
    hooks: &mut Map<String, Value>,
    event: &str,
    command: &str,
) -> io::Result<(usize, usize)> {
    let groups = hooks
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("hook event {event} must contain an array"),
            )
        })?;
    for (group_index, group) in groups.iter_mut().enumerate() {
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        for (handler_index, handler) in handlers.iter_mut().enumerate() {
            let is_managed = handler
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.contains(MANAGED_MARKER));
            if is_managed {
                *handler = managed_hook(command);
                return Ok((group_index, handler_index));
            }
        }
    }
    let group_index = groups.len();
    groups.push(json!({ "hooks": [managed_hook(command)] }));
    Ok((group_index, 0))
}

fn managed_hook(command: &str) -> Value {
    json!({
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_SECONDS,
    })
}

fn write_json(path: &Path, value: Value) -> io::Result<()> {
    let mut source = serde_json::to_vec_pretty(&value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    source.push(b'\n');
    atomic_write_with_parent(path, &source)
}

fn codex_hook_hash(event_label: &str, command: &str, timeout_seconds: u64) -> String {
    let identity = json!({
        "event_name": event_label,
        "hooks": [{
            "async": false,
            "command": command,
            "timeout": timeout_seconds,
            "type": "command",
        }],
    });
    let serialized = serde_json::to_vec(&identity).expect("static Codex hook identity serializes");
    format!("sha256:{:x}", Sha256::digest(serialized))
}

fn upsert_codex_trust_block(source: &str, key: &str, hash: &str) -> String {
    let header = format!("[hooks.state.\"{}\"]", escape_toml_string(key));
    let mut lines = source.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if let Some(start) = lines.iter().position(|line| line.trim() == header) {
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, line)| {
                let line = line.trim();
                line.starts_with('[') && line.ends_with(']')
            })
            .map(|(index, _)| index)
            .unwrap_or(lines.len());
        let enabled = lines[start + 1..end]
            .iter()
            .find(|line| line.trim_start().starts_with("enabled ="))
            .cloned();
        let mut replacement = vec![header.clone()];
        if let Some(enabled) = enabled {
            replacement.push(enabled);
        }
        replacement.push(format!("trusted_hash = \"{hash}\""));
        lines.splice(start..end, replacement);
    } else {
        if lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(format!("trusted_hash = \"{hash}\""));
    }
    format!("{}\n", lines.join("\n"))
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn managed_command(script_path: &Path, provider: &str) -> String {
    #[cfg(windows)]
    {
        format!(
            "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {} {provider}",
            powershell_quote(script_path.to_string_lossy().as_ref())
        )
    }
    #[cfg(not(windows))]
    {
        format!(
            "/bin/sh {} {provider}",
            shell_quote(script_path.to_string_lossy().as_ref())
        )
    }
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn managed_hook_file_name() -> &'static str {
    #[cfg(windows)]
    {
        "yttt-agent-hook.ps1"
    }
    #[cfg(not(windows))]
    {
        MANAGED_HOOK_FILE_NAME
    }
}

fn managed_hook_source() -> &'static str {
    #[cfg(windows)]
    {
        WINDOWS_HOOK_SOURCE
    }
    #[cfg(not(windows))]
    {
        POSIX_HOOK_SOURCE
    }
}

fn atomic_write_with_parent(path: &Path, source: &[u8]) -> io::Result<()> {
    if fs::read(path).ok().as_deref() == Some(source) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(path, source)
}

fn write_executable(path: &Path, source: &[u8]) -> io::Result<()> {
    atomic_write_with_parent(path, source)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn restrict_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn home_dir() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "user home is unavailable"))
}

fn with_context(context: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{context}: {error}"))
}

const POSIX_HOOK_SOURCE: &str = r#"#!/bin/sh
provider=$1
if [ -z "$provider" ] || [ -z "$YTTT_AGENT_HOOK_ENDPOINT" ] || [ -z "$YTTT_AGENT_HOOK_TOKEN" ] || [ -z "$YTTT_AGENT_HOOK_SCOPE" ]; then
  command -p cat >/dev/null 2>&1 || :
  exit 0
fi
curl --silent --show-error --max-time 2 --request POST \
  --header "Content-Type: application/json" \
  --header "X-Yttt-Agent-Hook-Token: $YTTT_AGENT_HOOK_TOKEN" \
  --header "X-Yttt-Agent-Hook-Scope: $YTTT_AGENT_HOOK_SCOPE" \
  --data-binary @- "$YTTT_AGENT_HOOK_ENDPOINT/hook/$provider" >/dev/null 2>&1 || :
exit 0
"#;

#[cfg(windows)]
const WINDOWS_HOOK_SOURCE: &str = r#"param([string]$Provider)
if (-not $Provider -or -not $env:YTTT_AGENT_HOOK_ENDPOINT -or -not $env:YTTT_AGENT_HOOK_TOKEN -or -not $env:YTTT_AGENT_HOOK_SCOPE) { [Console]::In.ReadToEnd() | Out-Null; exit 0 }
try {
  $body = [Console]::In.ReadToEnd()
  Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Method Post -Uri "$env:YTTT_AGENT_HOOK_ENDPOINT/hook/$Provider" -Headers @{ "X-Yttt-Agent-Hook-Token" = $env:YTTT_AGENT_HOOK_TOKEN; "X-Yttt-Agent-Hook-Scope" = $env:YTTT_AGENT_HOOK_SCOPE } -ContentType "application/json" -Body $body | Out-Null
} catch {}
exit 0
"#;

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn installs_all_managed_adapters_without_replacing_user_entries() {
        let temp = TempDir::new().unwrap();
        let config_paths = AppConfigPaths::from_config_dir(temp.path().join("yttt"));
        let home = temp.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo user"}]}]}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".codex/hooks.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo user"}]}]}}"#,
        )
        .unwrap();
        fs::write(home.join(".codex/config.toml"), "model = \"gpt-5\"\n").unwrap();

        install_managed_hooks_at(&config_paths, &home).unwrap();
        install_managed_hooks_at(&config_paths, &home).unwrap();

        let claude = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert!(claude.contains("\"theme\": \"dark\""));
        assert!(claude.contains("echo user"));
        assert_eq!(claude.matches(MANAGED_MARKER).count(), CLAUDE_EVENTS.len());

        let codex = fs::read_to_string(home.join(".codex/hooks.json")).unwrap();
        assert!(codex.contains("echo user"));
        assert_eq!(codex.matches(MANAGED_MARKER).count(), CODEX_EVENTS.len());
        let trust = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(trust.contains("model = \"gpt-5\""));
        assert_eq!(
            trust.matches("trusted_hash = \"sha256:").count(),
            CODEX_EVENTS.len()
        );

        assert!(
            home.join(".config/opencode/plugins/yttt-agent-status.js")
                .is_file()
        );
        assert!(
            home.join(".pi/agent/extensions/yttt-agent-status.ts")
                .is_file()
        );
        assert!(
            home.join(".omp/agent/extensions/yttt-agent-status.ts")
                .is_file()
        );
    }

    #[test]
    fn malformed_user_hook_config_is_not_overwritten() {
        let temp = TempDir::new().unwrap();
        let config_paths = AppConfigPaths::from_config_dir(temp.path().join("yttt"));
        let home = temp.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        let settings = home.join(".claude/settings.json");
        fs::write(&settings, "not-json").unwrap();

        let error = install_managed_hooks_at(&config_paths, &home).unwrap_err();
        assert!(error.to_string().contains("expected ident"));
        assert_eq!(fs::read_to_string(settings).unwrap(), "not-json");
        assert!(home.join(".codex/hooks.json").is_file());
    }
    #[test]
    fn codex_hash_matches_a_real_approved_hook() {
        let command = r#"/bin/sh "/tmp/orca-case-b-mCmCe6/agent-hooks/codex-hook.sh""#;
        assert_eq!(
            codex_hook_hash("pre_tool_use", command, 600),
            "sha256:bc013489dba495431d3790fda62ee5a7d907a7c491e29ad26238c3a5d6d2b163"
        );
    }

    #[test]
    fn refreshing_codex_trust_preserves_a_user_disabled_hook() {
        let key = "/tmp/hooks.json:stop:0:0";
        let source = format!(
            "[hooks.state.\"{key}\"]\nenabled = false\ntrusted_hash = \"old\"\n\n[projects.\"/tmp\"]\ntrust_level = \"trusted\"\n"
        );
        let updated = upsert_codex_trust_block(&source, key, "sha256:new");
        assert!(updated.contains("enabled = false"));
        assert!(updated.contains("trusted_hash = \"sha256:new\""));
        assert!(!updated.contains("trusted_hash = \"old\""));
        assert!(updated.contains("[projects.\"/tmp\"]"));
    }
}
