use std::{
    cmp::Reverse,
    collections::HashSet,
    env, fs,
    fs::File,
    io::{self, BufRead as _, BufReader, Read as _},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::Value;
use yttt_agent_core::AgentSessionMetadata;

use crate::config::default_layout::BuiltinAgent;

const MAX_SESSION_FILES: usize = 5_000;
const MAX_SESSION_RESULTS: usize = 100;
const MAX_TRANSCRIPT_BYTES: u64 = 512 * 1024;
const MAX_TRANSCRIPT_LINES: usize = 256;
const MAX_CLAUDE_INDEX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TITLE_CHARS: usize = 120;
const OPENCODE_MAX_RESULTS: &str = "200";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSession {
    pub provider: BuiltinAgent,
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub updated_at_ms: u64,
}

impl AgentSession {
    pub fn metadata(&self) -> AgentSessionMetadata {
        AgentSessionMetadata {
            session_id: Some(self.id.clone()),
            model: self.model.clone(),
            title: (!self.title.is_empty()).then(|| self.title.clone()),
            transcript_path: self
                .transcript_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSessionScanError {
    #[error("the home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("failed to scan {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to run {program}: {message}")]
    Command { program: String, message: String },
    #[error("invalid {provider} session list: {message}")]
    InvalidOutput {
        provider: &'static str,
        message: String,
    },
}

#[derive(Clone, Debug)]
struct AgentSessionRoots {
    codex: PathBuf,
    claude: PathBuf,
    pi: PathBuf,
    omp: PathBuf,
}

impl AgentSessionRoots {
    fn from_environment() -> Result<Self, AgentSessionScanError> {
        let home = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(AgentSessionScanError::HomeDirectoryUnavailable)?;
        let pi_override = env_path("PI_CODING_AGENT_SESSION_DIR");
        let pi_root = env_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".pi/agent"));
        let omp_root = env_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".omp/agent"));

        Ok(Self {
            codex: env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            claude: env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            pi: pi_override
                .clone()
                .unwrap_or_else(|| pi_root.join("sessions")),
            omp: pi_override.unwrap_or_else(|| omp_root.join("sessions")),
        })
    }
}

pub fn scan_agent_sessions(
    agent: BuiltinAgent,
    project_path: &Path,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let roots = AgentSessionRoots::from_environment()?;
    scan_agent_sessions_with_roots(agent, project_path, &roots)
}

fn scan_agent_sessions_with_roots(
    agent: BuiltinAgent,
    project_path: &Path,
    roots: &AgentSessionRoots,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let sessions = match agent {
        BuiltinAgent::Codex => scan_codex(&roots.codex.join("sessions"), project_path)?,
        BuiltinAgent::Claude => scan_claude(&roots.claude.join("projects"), project_path)?,
        BuiltinAgent::OpenCode => scan_opencode(project_path)?,
        BuiltinAgent::Pi => scan_pi_family(&roots.pi, project_path, BuiltinAgent::Pi)?,
        BuiltinAgent::OhMyPi => scan_pi_family(&roots.omp, project_path, BuiltinAgent::OhMyPi)?,
    };
    Ok(finalize_sessions(sessions))
}

fn scan_codex(
    sessions_root: &Path,
    project_path: &Path,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let files = collect_session_files(sessions_root, true)?;
    Ok(files
        .into_iter()
        .filter_map(|file| parse_codex_session(&file, project_path))
        .collect())
}

fn parse_codex_session(file: &SessionFile, project_path: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut title = None;
    let mut model = None;
    for value in transcript_values(&file.path) {
        let event_type = value.get("type").and_then(Value::as_str);
        if event_type == Some("session_meta") {
            let payload = value.get("payload")?;
            id = string_at(payload, &["session_id", "id"]);
            cwd = string_at(payload, &["cwd"]);
        } else if title.is_none() {
            title = codex_title(&value);
        }
        model = model.or_else(|| session_model(&value));
    }
    let cwd = cwd?;
    if !belongs_to_project(Path::new(&cwd), project_path) {
        return None;
    }
    let id = id?;
    Some(AgentSession {
        provider: BuiltinAgent::Codex,
        title: title.unwrap_or_default(),
        model,
        id,
        transcript_path: Some(file.path.clone()),
        updated_at_ms: file.updated_at_ms,
    })
}

fn codex_title(value: &Value) -> Option<String> {
    let payload = value.get("payload")?;
    let payload_type = payload.get("type").and_then(Value::as_str);
    if matches!(payload_type, Some("thread_name" | "session_name")) {
        return string_at(payload, &["name", "title"]).and_then(clean_title);
    }
    if payload_type == Some("user_message") {
        return payload
            .get("message")
            .and_then(value_text)
            .and_then(clean_title);
    }
    let role = payload.get("role").and_then(Value::as_str);
    if role == Some("user") {
        return payload
            .get("content")
            .and_then(value_text)
            .and_then(clean_title);
    }
    None
}

fn scan_claude(
    projects_root: &Path,
    project_path: &Path,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    if !projects_root.exists() {
        return Ok(Vec::new());
    }
    let directories = fs::read_dir(projects_root).map_err(|source| AgentSessionScanError::Io {
        path: projects_root.to_path_buf(),
        source,
    })?;
    let mut sessions = Vec::new();
    let mut indexed_ids = HashSet::new();
    for directory in directories {
        let directory = directory.map_err(|source| AgentSessionScanError::Io {
            path: projects_root.to_path_buf(),
            source,
        })?;
        let file_type = directory
            .file_type()
            .map_err(|source| AgentSessionScanError::Io {
                path: directory.path(),
                source,
            })?;
        if !file_type.is_dir() {
            continue;
        }
        let directory_path = directory.path();
        let index_path = directory_path.join("sessions-index.json");
        if index_path.is_file() {
            for session in parse_claude_index(&index_path, project_path)? {
                indexed_ids.insert(session.id.clone());
                sessions.push(session);
            }
        }
        let files = collect_session_files(&directory_path, false)?;
        sessions.extend(files.into_iter().filter_map(|file| {
            let session = parse_claude_transcript(&file, project_path)?;
            (!indexed_ids.contains(&session.id)).then_some(session)
        }));
    }
    Ok(sessions)
}

#[derive(Deserialize)]
struct ClaudeSessionIndex {
    #[serde(default)]
    entries: Vec<ClaudeSessionIndexEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeSessionIndexEntry {
    session_id: String,
    full_path: PathBuf,
    file_mtime: u64,
    #[serde(default)]
    first_prompt: String,
    #[serde(default)]
    summary: String,
    project_path: PathBuf,
    #[serde(default)]
    is_sidechain: bool,
}

fn parse_claude_index(
    index_path: &Path,
    project_path: &Path,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let file = File::open(index_path).map_err(|source| AgentSessionScanError::Io {
        path: index_path.to_path_buf(),
        source,
    })?;
    if file
        .metadata()
        .map(|metadata| metadata.len() > MAX_CLAUDE_INDEX_BYTES)
        .unwrap_or(true)
    {
        return Ok(Vec::new());
    }
    let index: ClaudeSessionIndex =
        serde_json::from_reader(file).map_err(|error| AgentSessionScanError::InvalidOutput {
            provider: "Claude Code",
            message: format!("{}: {error}", index_path.display()),
        })?;
    Ok(index
        .entries
        .into_iter()
        .filter(|entry| {
            !entry.is_sidechain && belongs_to_project(&entry.project_path, project_path)
        })
        .map(|entry| {
            let title = clean_title(&entry.summary)
                .or_else(|| clean_title(&entry.first_prompt))
                .unwrap_or_default();
            AgentSession {
                provider: BuiltinAgent::Claude,
                id: entry.session_id,
                title,
                model: None,
                transcript_path: Some(entry.full_path),
                updated_at_ms: entry.file_mtime,
            }
        })
        .collect())
}

fn parse_claude_transcript(file: &SessionFile, project_path: &Path) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut title = None;
    let mut model = None;
    for value in transcript_values(&file.path) {
        id = id.or_else(|| string_at(&value, &["sessionId"]));
        cwd = cwd.or_else(|| string_at(&value, &["cwd"]));
        if title.is_none() && value.get("type").and_then(Value::as_str) == Some("user") {
            title = user_message_title(&value);
        }
        model = model.or_else(|| session_model(&value));
        if id.is_some() && cwd.is_some() && title.is_some() && model.is_some() {
            break;
        }
    }
    let cwd = cwd?;
    if !belongs_to_project(Path::new(&cwd), project_path) {
        return None;
    }
    let id = id?;
    Some(AgentSession {
        provider: BuiltinAgent::Claude,
        title: title.unwrap_or_default(),
        model,
        id,
        transcript_path: Some(file.path.clone()),
        updated_at_ms: file.updated_at_ms,
    })
}

fn scan_pi_family(
    sessions_root: &Path,
    project_path: &Path,
    provider: BuiltinAgent,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let files = collect_session_files(sessions_root, true)?;
    Ok(files
        .into_iter()
        .filter_map(|file| parse_pi_family_session(&file, project_path, provider))
        .collect())
}

fn parse_pi_family_session(
    file: &SessionFile,
    project_path: &Path,
    provider: BuiltinAgent,
) -> Option<AgentSession> {
    let mut id = None;
    let mut cwd = None;
    let mut title = None;
    let mut first_prompt = None;
    let mut model = None;
    for value in transcript_values(&file.path) {
        match value.get("type").and_then(Value::as_str) {
            Some("session") => {
                id = string_at(&value, &["id"]);
                cwd = string_at(&value, &["cwd"]);
            }
            Some("title") => {
                title = string_at(&value, &["title"]).and_then(clean_title);
            }
            Some("message") if first_prompt.is_none() => {
                first_prompt = user_message_title(&value);
            }
            _ => {}
        }
        model = model.or_else(|| session_model(&value));
        if id.is_some()
            && cwd.is_some()
            && (title.is_some() || first_prompt.is_some())
            && model.is_some()
        {
            break;
        }
    }
    let cwd = cwd?;
    if !belongs_to_project(Path::new(&cwd), project_path) {
        return None;
    }
    let id = id?;
    Some(AgentSession {
        provider,
        title: title.or(first_prompt).unwrap_or_default(),
        model,
        id,
        transcript_path: Some(file.path.clone()),
        updated_at_ms: file.updated_at_ms,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeSession {
    id: String,
    title: String,
    updated: u64,
    directory: PathBuf,
}

fn scan_opencode(project_path: &Path) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let output = Command::new(BuiltinAgent::OpenCode.command())
        .args([
            "--pure",
            "session",
            "list",
            "--format",
            "json",
            "--max-count",
            OPENCODE_MAX_RESULTS,
        ])
        .current_dir(project_path)
        .output()
        .map_err(|error| AgentSessionScanError::Command {
            program: BuiltinAgent::OpenCode.command().to_string(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(AgentSessionScanError::Command {
            program: BuiltinAgent::OpenCode.command().to_string(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    parse_opencode_output(&output.stdout, project_path)
}

fn parse_opencode_output(
    output: &[u8],
    project_path: &Path,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let entries: Vec<OpenCodeSession> =
        serde_json::from_slice(output).map_err(|error| AgentSessionScanError::InvalidOutput {
            provider: "OpenCode",
            message: error.to_string(),
        })?;
    Ok(entries
        .into_iter()
        .filter(|entry| belongs_to_project(&entry.directory, project_path))
        .map(|entry| AgentSession {
            provider: BuiltinAgent::OpenCode,
            title: clean_title(&entry.title).unwrap_or_default(),
            model: None,
            id: entry.id,
            transcript_path: None,
            updated_at_ms: entry.updated,
        })
        .collect())
}

#[derive(Clone, Debug)]
struct SessionFile {
    path: PathBuf,
    updated_at_ms: u64,
}

fn collect_session_files(
    root: &Path,
    recursive: bool,
) -> Result<Vec<SessionFile>, AgentSessionScanError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory).map_err(|source| AgentSessionScanError::Io {
            path: directory.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| AgentSessionScanError::Io {
                path: directory.clone(),
                source,
            })?;
            let file_type = entry
                .file_type()
                .map_err(|source| AgentSessionScanError::Io {
                    path: entry.path(),
                    source,
                })?;
            if file_type.is_dir() {
                if recursive && entry.file_name() != "subagents" {
                    directories.push(entry.path());
                }
                continue;
            }
            if !file_type.is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            let updated_at_ms = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .map(system_time_millis)
                .unwrap_or_default();
            files.push(SessionFile {
                path: entry.path(),
                updated_at_ms,
            });
        }
    }
    files.sort_unstable_by_key(|file| Reverse(file.updated_at_ms));
    files.truncate(MAX_SESSION_FILES);
    Ok(files)
}

fn transcript_values(path: &Path) -> impl Iterator<Item = Value> {
    let reader = File::open(path)
        .map(|file| BufReader::new(file.take(MAX_TRANSCRIPT_BYTES)))
        .ok();
    reader
        .into_iter()
        .flat_map(|reader| reader.lines().take(MAX_TRANSCRIPT_LINES))
        .filter_map(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
}

fn value_text(value: &Value) -> Option<&str> {
    if let Some(text) = value.as_str() {
        return Some(text);
    }
    value.as_array()?.iter().find_map(|part| {
        part.get("text")
            .or_else(|| part.get("input_text"))
            .and_then(Value::as_str)
    })
}

fn user_message_title(value: &Value) -> Option<String> {
    (value.pointer("/message/role").and_then(Value::as_str) == Some("user"))
        .then(|| value.pointer("/message/content"))
        .flatten()
        .and_then(value_text)
        .and_then(clean_title)
}

fn session_model(value: &Value) -> Option<String> {
    string_at(value, &["model"])
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| string_at(payload, &["model"]))
        })
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| string_at(message, &["model"]))
        })
        .and_then(clean_title)
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn clean_title(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref();
    let mut title = String::with_capacity(value.len().min(MAX_TITLE_CHARS));
    let mut title_chars = 0;
    let mut pending_space = false;
    for character in value.trim().chars() {
        if character.is_whitespace() {
            pending_space = !title.is_empty();
            continue;
        }
        if pending_space && title_chars < MAX_TITLE_CHARS {
            title.push(' ');
            title_chars += 1;
        }
        pending_space = false;
        if title_chars >= MAX_TITLE_CHARS {
            break;
        }
        title.push(character);
        title_chars += 1;
    }
    (!title.is_empty()).then_some(title)
}

fn belongs_to_project(session_path: &Path, project_path: &Path) -> bool {
    session_path == project_path || session_path.starts_with(project_path)
}

fn finalize_sessions(mut sessions: Vec<AgentSession>) -> Vec<AgentSession> {
    sessions.sort_unstable_by_key(|session| Reverse(session.updated_at_ms));
    let mut ids = HashSet::new();
    sessions.retain(|session| ids.insert((session.provider.id(), session.id.clone())));
    sessions.truncate(MAX_SESSION_RESULTS);
    sessions
}

fn system_time_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(root: &Path) -> AgentSessionRoots {
        AgentSessionRoots {
            codex: root.join("codex"),
            claude: root.join("claude"),
            pi: root.join("pi"),
            omp: root.join("omp"),
        }
    }

    #[test]
    fn scans_omp_sessions_for_the_selected_project() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        fs::create_dir_all(roots.omp.join("encoded-project")).unwrap();
        let transcript = roots.omp.join("encoded-project/session.jsonl");
        fs::write(
            &transcript,
            format!(
                "{{\"type\":\"title\",\"title\":\"Fix the sidebar\"}}\n{{\"type\":\"session\",\"id\":\"session-1\",\"cwd\":{:?}}}\n",
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let sessions =
            scan_agent_sessions_with_roots(BuiltinAgent::OhMyPi, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "session-1");
        assert_eq!(sessions[0].title, "Fix the sidebar");
        assert_eq!(
            sessions[0].transcript_path.as_deref(),
            Some(transcript.as_path())
        );
    }

    #[test]
    fn omp_blank_title_falls_back_to_first_user_prompt_and_reads_model() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        fs::create_dir_all(roots.omp.join("encoded-project")).unwrap();
        fs::write(
            roots.omp.join("encoded-project/session.jsonl"),
            format!(
                concat!(
                    "{{\"type\":\"title\",\"title\":\"\"}}\n",
                    "{{\"type\":\"session\",\"id\":\"session-1\",\"cwd\":{:?}}}\n",
                    "{{\"type\":\"model_change\",\"model\":\"openai-codex/gpt-5.6-sol\"}}\n",
                    "{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":[",
                    "{{\"type\":\"text\",\"text\":\"Improve the session list\"}}]}}}}\n"
                ),
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let sessions =
            scan_agent_sessions_with_roots(BuiltinAgent::OhMyPi, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Improve the session list");
        assert_eq!(
            sessions[0].model.as_deref(),
            Some("openai-codex/gpt-5.6-sol")
        );
        assert_eq!(
            sessions[0].metadata().model.as_deref(),
            Some("openai-codex/gpt-5.6-sol")
        );
    }

    #[test]
    fn untitled_omp_session_does_not_use_its_id_as_a_title() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        fs::create_dir_all(roots.omp.join("encoded-project")).unwrap();
        fs::write(
            roots.omp.join("encoded-project/session.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"opaque-session-id\",\"cwd\":{:?}}}\n",
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let sessions =
            scan_agent_sessions_with_roots(BuiltinAgent::OhMyPi, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].title.is_empty());
        assert!(sessions[0].metadata().title.is_none());
    }

    #[test]
    fn scans_codex_metadata_and_first_user_message() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        let session_directory = roots.codex.join("sessions/2026/08/05");
        fs::create_dir_all(&session_directory).unwrap();
        fs::write(
            session_directory.join("rollout.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-1\",\"cwd\":{:?}}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"Implement session restore\"}}}}\n",
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let sessions =
            scan_agent_sessions_with_roots(BuiltinAgent::Codex, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "codex-1");
        assert_eq!(sessions[0].title, "Implement session restore");
    }

    #[test]
    fn excludes_sessions_from_other_projects() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        fs::create_dir_all(roots.pi.join("encoded-project")).unwrap();
        fs::write(
            roots.pi.join("encoded-project/session.jsonl"),
            "{\"type\":\"session\",\"id\":\"session-1\",\"cwd\":\"/other\"}\n",
        )
        .unwrap();

        let sessions = scan_agent_sessions_with_roots(
            BuiltinAgent::Pi,
            temporary.path().join("project").as_path(),
            &roots,
        )
        .unwrap();

        assert!(sessions.is_empty());
    }

    #[test]
    fn parses_claude_session_index() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        let index_directory = roots.claude.join("projects/encoded-project");
        fs::create_dir_all(&index_directory).unwrap();
        fs::write(
            index_directory.join("sessions-index.json"),
            format!(
                "{{\"entries\":[{{\"sessionId\":\"claude-1\",\"fullPath\":\"/tmp/claude-1.jsonl\",\"fileMtime\":42,\"firstPrompt\":\"first\",\"summary\":\"Session summary\",\"projectPath\":{:?},\"isSidechain\":false}}]}}",
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let sessions =
            scan_agent_sessions_with_roots(BuiltinAgent::Claude, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Session summary");
        assert_eq!(sessions[0].updated_at_ms, 42);
    }

    #[test]
    fn parses_opencode_json_output() {
        let project = Path::new("/workspace/project");
        let output = br#"[{"id":"ses_1","title":"Refactor auth","updated":99,"directory":"/workspace/project"}]"#;

        let sessions = parse_opencode_output(output, project).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].provider, BuiltinAgent::OpenCode);
        assert_eq!(sessions[0].id, "ses_1");
        assert_eq!(sessions[0].title, "Refactor auth");
    }
}
