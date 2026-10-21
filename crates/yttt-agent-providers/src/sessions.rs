use std::{
    collections::{HashMap, HashSet},
    fs,
    fs::File,
    io::{self, BufRead as _, BufReader, Read as _},
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::Value;
use yttt_agent_core::AgentSessionMetadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionProvider {
    Codex,
    Claude,
    Grok,
    OpenCode,
    Pi,
    OhMyPi,
}

impl SessionProvider {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "grok" => Some(Self::Grok),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            "omp" => Some(Self::OhMyPi),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::OhMyPi => "omp",
        }
    }
    fn command(self) -> &'static str {
        self.id()
    }
}

pub struct AgentSessionRoots {
    pub codex: PathBuf,
    pub claude: PathBuf,
    pub grok: PathBuf,
    pub pi: PathBuf,
    pub omp: PathBuf,
}

impl AgentSessionRoots {
    pub fn native() -> Option<Self> {
        let env_path = |name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let home = env_path("HOME").or_else(|| env_path("USERPROFILE"))?;
        let pi_override = env_path("PI_CODING_AGENT_SESSION_DIR");
        Some(Self {
            codex: env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            claude: env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            grok: env_path("GROK_HOME")
                .unwrap_or_else(|| home.join(".grok"))
                .join("sessions"),
            pi: pi_override.clone().unwrap_or_else(|| {
                env_path("PI_CODING_AGENT_DIR")
                    .unwrap_or_else(|| home.join(".pi/agent"))
                    .join("sessions")
            }),
            omp: pi_override.unwrap_or_else(|| {
                env_path("PI_CODING_AGENT_DIR")
                    .unwrap_or_else(|| home.join(".omp/agent"))
                    .join("sessions")
            }),
        })
    }
}

/// Checks saved OMP IDs independently of project-scoped history discovery.
/// I/O errors are not evidence that a conversation is missing.
pub fn omp_session_exists(root: &Path, session_id: &str) -> io::Result<bool> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid OMP session ID",
        ));
    }
    let suffix = format!("_{session_id}.jsonl");
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound && directory == root => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                directories.push(entry.path());
            } else if entry.file_name().to_string_lossy().ends_with(&suffix)
                && entry.metadata()?.is_file()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

// Optional display metadata must not require reading an entire conversation.
// Missing session identity is still searched for beyond this many records.
const DISPLAY_METADATA_RECORDS: usize = 256;
const MAX_TITLE_CHARS: usize = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSession {
    pub provider: SessionProvider,
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

#[derive(Debug, Default)]
pub struct AgentSessionScan {
    pub sessions: Vec<AgentSession>,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSessionScanError {
    #[error("invalid project path {path}: {source}")]
    ProjectPath {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
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

pub fn scan_agent_sessions_for_agents_with_roots(
    agents: &[SessionProvider],
    project_path: &Path,
    roots: &AgentSessionRoots,
    allow_native_commands: bool,
) -> Result<AgentSessionScan, AgentSessionScanError> {
    let mut context = ScanContext::new(project_path)?;
    let mut sessions = Vec::new();
    let mut scanned_agents = HashSet::new();
    for &agent in agents {
        if !scanned_agents.insert(agent.id()) {
            continue;
        }
        let result = match agent {
            SessionProvider::Codex => Ok(scan_transcripts(
                &roots.codex.join("sessions"),
                &mut context,
                agent,
            )),
            SessionProvider::Claude => {
                Ok(scan_claude(&roots.claude.join("projects"), &mut context))
            }
            SessionProvider::Grok => Ok(scan_grok(&roots.grok, &mut context)),
            SessionProvider::OpenCode if allow_native_commands => scan_opencode(&mut context),
            SessionProvider::OpenCode => {
                context
                    .warnings
                    .push("opencode: native session commands are disabled for this profile".into());
                Ok(Vec::new())
            }
            SessionProvider::Pi => Ok(scan_transcripts(&roots.pi, &mut context, agent)),
            SessionProvider::OhMyPi => Ok(scan_transcripts(&roots.omp, &mut context, agent)),
        };
        match result {
            Ok(found) => sessions.extend(found),
            Err(error) => context.warnings.push(format!("{}: {error}", agent.id())),
        }
    }
    Ok(AgentSessionScan {
        sessions: finalize_sessions(sessions),
        warnings: context.warnings,
    })
}

#[cfg(test)]
fn scan_agent_sessions_with_roots(
    agent: SessionProvider,
    project_path: &Path,
    roots: &AgentSessionRoots,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    scan_agent_sessions_for_agents_with_roots(&[agent], project_path, roots, false).map(|scan| {
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
        scan.sessions
    })
}

fn scan_transcripts(
    root: &Path,
    context: &mut ScanContext,
    provider: SessionProvider,
) -> Vec<AgentSession> {
    collect_files(
        root,
        |path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        },
        context,
    )
    .into_iter()
    .filter_map(|file| match parse_transcript(&file, context, provider) {
        Ok(session) => session,
        Err(error) => {
            context.warnings.push(error.to_string());
            None
        }
    })
    .collect()
}

fn parse_transcript(
    file: &SessionFile,
    context: &mut ScanContext,
    provider: SessionProvider,
) -> Result<Option<AgentSession>, AgentSessionScanError> {
    let mut records = TranscriptValues::open(&file.path)?;
    let mut id = None;
    let mut cwd = None;
    let mut title = None;
    let mut first_prompt = None;
    let mut model = None;
    for (index, value) in records.by_ref().enumerate() {
        let kind = value.get("type").and_then(Value::as_str);
        match provider {
            SessionProvider::Codex => {
                if kind == Some("session_meta") {
                    if let Some(payload) = value.get("payload") {
                        id = string_at(payload, &["session_id", "id"]);
                        cwd = string_at(payload, &["cwd"]);
                    }
                } else if title.is_none() {
                    title = codex_title(&value);
                }
            }
            SessionProvider::Claude => {
                if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                    return Ok(None);
                }
                id = id.or_else(|| string_at(&value, &["sessionId"]));
                cwd = cwd.or_else(|| string_at(&value, &["cwd"]));
                if title.is_none() && kind == Some("user") {
                    title = user_message_title(&value);
                }
            }
            SessionProvider::Pi | SessionProvider::OhMyPi => match kind {
                Some("session") => {
                    id = string_at(&value, &["id"]);
                    cwd = string_at(&value, &["cwd"]);
                }
                Some("title") => title = string_at(&value, &["title"]).and_then(clean_title),
                Some("message") if first_prompt.is_none() => {
                    first_prompt = user_message_title(&value);
                }
                _ => {}
            },
            SessionProvider::Grok | SessionProvider::OpenCode => unreachable!(),
        }
        if let Some(cwd) = &cwd
            && !context.belongs_to_project(Path::new(cwd))
        {
            return Ok(None);
        }
        model = model.or_else(|| session_model(&value));
        if id.is_some()
            && cwd.is_some()
            && (((title.is_some() || first_prompt.is_some()) && model.is_some())
                || index + 1 >= DISPLAY_METADATA_RECORDS)
        {
            break;
        }
    }
    if let Some(error) = records.first_error {
        context.warn(&file.path, error);
    }
    let Some(id) = id
        .filter(|id| !id.trim().is_empty())
        .filter(|_| cwd.is_some())
    else {
        context.warn(&file.path, "missing session ID or working directory");
        return Ok(None);
    };
    Ok(Some(AgentSession {
        provider,
        title: title.or(first_prompt).unwrap_or_default(),
        model,
        id,
        transcript_path: Some(file.path.clone()),
        updated_at_ms: file.updated_at_ms,
    }))
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

fn scan_claude(projects_root: &Path, context: &mut ScanContext) -> Vec<AgentSession> {
    let files = collect_files(
        projects_root,
        |path| {
            path.file_name()
                .is_some_and(|name| name == "sessions-index.json")
                || path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
        },
        context,
    );
    let mut sessions = HashMap::<String, AgentSession>::new();
    for file in files.iter().filter(|file| {
        file.path
            .file_name()
            .is_some_and(|name| name == "sessions-index.json")
    }) {
        match parse_claude_index(&file.path, context) {
            Ok(indexed) => {
                for session in indexed {
                    match sessions.entry(session.id.clone()) {
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(session);
                        }
                        std::collections::hash_map::Entry::Occupied(mut entry)
                            if session.updated_at_ms > entry.get().updated_at_ms =>
                        {
                            entry.insert(session);
                        }
                        _ => {}
                    }
                }
            }
            Err(error) => context.warnings.push(error.to_string()),
        }
    }
    for file in files.iter().filter(|file| {
        file.path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
    }) {
        match parse_transcript(file, context, SessionProvider::Claude) {
            Ok(Some(session)) => {
                if let Some(indexed) = sessions.get_mut(&session.id) {
                    indexed.updated_at_ms = indexed.updated_at_ms.max(session.updated_at_ms);
                    indexed.model = session.model;
                    indexed.transcript_path = session.transcript_path;
                    if indexed.title.is_empty() {
                        indexed.title = session.title;
                    }
                } else {
                    sessions.insert(session.id.clone(), session);
                }
            }
            Ok(None) => {}
            Err(error) => context.warnings.push(error.to_string()),
        }
    }
    sessions.into_values().collect()
}

#[derive(Deserialize)]
struct ClaudeSessionIndex {
    #[serde(default)]
    entries: Vec<Value>,
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
    context: &mut ScanContext,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let file = File::open(index_path).map_err(|source| AgentSessionScanError::Io {
        path: index_path.to_path_buf(),
        source,
    })?;
    let index: ClaudeSessionIndex =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| {
            AgentSessionScanError::InvalidOutput {
                provider: "Claude Code",
                message: format!("{}: {error}", index_path.display()),
            }
        })?;
    Ok(index
        .entries
        .into_iter()
        .filter_map(
            |entry| match serde_json::from_value::<ClaudeSessionIndexEntry>(entry) {
                Ok(entry)
                    if !entry.is_sidechain && context.belongs_to_project(&entry.project_path) =>
                {
                    Some(entry)
                }
                Ok(_) => None,
                Err(error) => {
                    context.warn(index_path, error);
                    None
                }
            },
        )
        .map(|entry| {
            let title = clean_title(&entry.summary)
                .or_else(|| clean_title(&entry.first_prompt))
                .unwrap_or_default();
            AgentSession {
                provider: SessionProvider::Claude,
                id: entry.session_id,
                title,
                model: None,
                transcript_path: Some(entry.full_path),
                updated_at_ms: entry.file_mtime,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct GrokSessionSummary {
    info: GrokSessionInfo,
    #[serde(default)]
    generated_title: String,
    #[serde(default)]
    session_summary: String,
    current_model_id: Option<String>,
}

#[derive(Deserialize)]
struct GrokSessionInfo {
    id: String,
    cwd: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrokSessionKind {
    Root,
    Subagent,
}

pub fn grok_session_kind(transcript: &Path, session_id: &str) -> Option<GrokSessionKind> {
    const MAX_SUMMARY_BYTES: u64 = 1024 * 1024;
    let path = transcript.parent()?.join("summary.json");
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SUMMARY_BYTES {
        return None;
    }
    let mut source = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_SUMMARY_BYTES + 1)
        .read_to_end(&mut source)
        .ok()?;
    if source.len() as u64 > MAX_SUMMARY_BYTES {
        return None;
    }
    let summary: Value = serde_json::from_slice(&source).ok()?;
    if summary.get("info")?.get("id")?.as_str()? != session_id {
        return None;
    }
    match summary.get("session_kind") {
        None | Some(Value::Null) => Some(GrokSessionKind::Root),
        Some(Value::String(kind)) if kind == "subagent" => Some(GrokSessionKind::Subagent),
        _ => None,
    }
}

fn scan_grok(root: &Path, context: &mut ScanContext) -> Vec<AgentSession> {
    collect_files(
        root,
        |path| path.file_name().is_some_and(|name| name == "summary.json"),
        context,
    )
    .into_iter()
    .filter_map(|file| {
        let result = File::open(&file.path)
            .map(BufReader::new)
            .map_err(|error| error.to_string())
            .and_then(|reader| {
                serde_json::from_reader::<_, GrokSessionSummary>(reader)
                    .map_err(|error| error.to_string())
            });
        let summary = match result {
            Ok(summary) => summary,
            Err(error) => {
                context.warn(&file.path, error);
                return None;
            }
        };
        if !context.belongs_to_project(&summary.info.cwd) {
            return None;
        }
        let id = summary.info.id.trim();
        if id.is_empty() {
            context.warn(&file.path, "missing session ID");
            return None;
        }
        Some(AgentSession {
            provider: SessionProvider::Grok,
            id: id.to_string(),
            title: clean_title(summary.generated_title)
                .or_else(|| clean_title(summary.session_summary))
                .unwrap_or_default(),
            model: summary.current_model_id.and_then(clean_title),
            transcript_path: None,
            updated_at_ms: file.updated_at_ms,
        })
    })
    .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenCodeSession {
    id: String,
    title: String,
    updated: u64,
    directory: PathBuf,
}

fn scan_opencode(context: &mut ScanContext) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    let output = Command::new(SessionProvider::OpenCode.command())
        .args([
            "--pure",
            "session",
            "list",
            "--format",
            "json",
            // Omitting this option still limits OpenCode to 100 sessions.
            "--max-count",
            "-1",
        ])
        .current_dir(&context.project_path)
        .output()
        .map_err(|error| AgentSessionScanError::Command {
            program: SessionProvider::OpenCode.command().to_string(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(AgentSessionScanError::Command {
            program: SessionProvider::OpenCode.command().to_string(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    parse_opencode_output(&output.stdout, context)
}

fn parse_opencode_output(
    output: &[u8],
    context: &mut ScanContext,
) -> Result<Vec<AgentSession>, AgentSessionScanError> {
    // The native CLI exits successfully with no output for an empty store.
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let entries: Vec<Value> =
        serde_json::from_slice(output).map_err(|error| AgentSessionScanError::InvalidOutput {
            provider: "OpenCode",
            message: error.to_string(),
        })?;
    Ok(entries
        .into_iter()
        .filter_map(
            |entry| match serde_json::from_value::<OpenCodeSession>(entry) {
                Ok(entry) if context.belongs_to_project(&entry.directory) => Some(entry),
                Ok(_) => None,
                Err(error) => {
                    context.warnings.push(format!("opencode: {error}"));
                    None
                }
            },
        )
        .map(|entry| AgentSession {
            provider: SessionProvider::OpenCode,
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

fn collect_files(
    root: &Path,
    accepts: impl Fn(&Path) -> bool,
    context: &mut ScanContext,
) -> Vec<SessionFile> {
    let mut directories = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        if !visited.insert(normalize_path(&directory)) {
            continue;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound && directory == root => continue,
            Err(error) => {
                context.warn(&directory, error);
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    context.warn(&directory, error);
                    continue;
                }
            };
            let path = entry.path();
            if entry.file_name() == "subagents" {
                continue;
            }
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    context.warn(&path, error);
                    continue;
                }
            };
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() && accepts(&path) {
                files.push(SessionFile {
                    path,
                    updated_at_ms: metadata
                        .modified()
                        .map(system_time_millis)
                        .unwrap_or_default(),
                });
            }
        }
    }
    files
}

struct TranscriptValues {
    reader: BufReader<File>,
    line: String,
    first_error: Option<String>,
    finished: bool,
}

impl TranscriptValues {
    fn open(path: &Path) -> Result<Self, AgentSessionScanError> {
        let file = File::open(path).map_err(|source| AgentSessionScanError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            reader: BufReader::new(file),
            line: String::new(),
            first_error: None,
            finished: false,
        })
    }
}

impl Iterator for TranscriptValues {
    type Item = Value;

    fn next(&mut self) -> Option<Value> {
        while !self.finished {
            self.line.clear();
            match self.reader.read_line(&mut self.line) {
                Ok(0) => self.finished = true,
                Ok(_) if self.line.trim().is_empty() => {}
                Ok(_) => match serde_json::from_str(&self.line) {
                    Ok(value) => return Some(value),
                    Err(error) => {
                        self.first_error.get_or_insert_with(|| error.to_string());
                    }
                },
                Err(error) => {
                    self.first_error.get_or_insert_with(|| error.to_string());
                    self.finished = true;
                }
            }
        }
        None
    }
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

struct ScanContext {
    project_path: PathBuf,
    project_matches: HashMap<PathBuf, bool>,
    warnings: Vec<String>,
}

impl ScanContext {
    fn new(project_path: &Path) -> Result<Self, AgentSessionScanError> {
        let absolute = std::path::absolute(project_path).map_err(|source| {
            AgentSessionScanError::ProjectPath {
                path: project_path.to_path_buf(),
                source,
            }
        })?;
        Ok(Self {
            project_path: normalize_path(&absolute),
            project_matches: HashMap::new(),
            warnings: Vec::new(),
        })
    }

    fn belongs_to_project(&mut self, path: &Path) -> bool {
        if let Some(matches) = self.project_matches.get(path) {
            return *matches;
        }
        let matches = path.is_absolute() && normalize_path(path).starts_with(&self.project_path);
        self.project_matches.insert(path.to_path_buf(), matches);
        matches
    }

    fn warn(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.warnings.push(format!("{}: {error}", path.display()));
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    // Resolve an existing ancestor too: a historical cwd may have been deleted
    // beneath a still-valid directory alias.
    let resolved = path.ancestors().skip(1).find_map(|ancestor| {
        fs::canonicalize(ancestor).ok().map(|canonical| {
            canonical.join(
                path.strip_prefix(ancestor)
                    .expect("ancestor is a path prefix"),
            )
        })
    });
    let mut normalized = PathBuf::new();
    for component in resolved.as_deref().unwrap_or(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn finalize_sessions(mut sessions: Vec<AgentSession>) -> Vec<AgentSession> {
    sessions.sort_unstable_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| left.provider.id().cmp(right.provider.id()))
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut ids = HashSet::new();
    sessions.retain(|session| ids.insert((session.provider.id(), session.id.clone())));
    sessions
}

fn system_time_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_session_kind_requires_matching_bounded_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let transcript = temp.path().join("updates.jsonl");
        let summary = temp.path().join("summary.json");
        assert_eq!(grok_session_kind(&transcript, "root"), None);
        for (kind, expected) in [
            (Value::Null, Some(GrokSessionKind::Root)),
            (
                Value::String("subagent".into()),
                Some(GrokSessionKind::Subagent),
            ),
            (Value::String("unknown".into()), None),
        ] {
            fs::write(
                &summary,
                serde_json::to_vec(&serde_json::json!({
                    "info":{"id":"root"}, "session_kind":kind,
                }))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(grok_session_kind(&transcript, "root"), expected);
            assert_eq!(grok_session_kind(&transcript, "other"), None);
        }
        fs::write(&summary, br#"{"info":{"id":"root"}}"#).unwrap();
        assert_eq!(
            grok_session_kind(&transcript, "root"),
            Some(GrokSessionKind::Root)
        );
        fs::write(&summary, "{").unwrap();
        assert_eq!(grok_session_kind(&transcript, "root"), None);
        fs::write(&summary, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert_eq!(grok_session_kind(&transcript, "root"), None);
    }

    #[test]
    fn omp_session_lookup_distinguishes_missing_history_from_scan_errors() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sessions");
        assert!(!omp_session_exists(&root, "saved").unwrap());
        fs::create_dir_all(root.join("moved-project")).unwrap();
        fs::write(root.join("moved-project/date_saved.jsonl"), "{}\n").unwrap();
        assert!(omp_session_exists(&root, "saved").unwrap());
        assert!(!omp_session_exists(&root, "save").unwrap());
        fs::remove_file(root.join("moved-project/date_saved.jsonl")).unwrap();
        assert!(!omp_session_exists(&root, "saved").unwrap());
        let not_a_directory = temp.path().join("blocked");
        fs::write(&not_a_directory, "").unwrap();
        assert!(omp_session_exists(&not_a_directory, "saved").is_err());
    }

    fn roots(root: &Path) -> AgentSessionRoots {
        AgentSessionRoots {
            codex: root.join("codex"),
            claude: root.join("claude"),
            grok: root.join("grok"),
            pi: root.join("pi"),
            omp: root.join("omp"),
        }
    }

    #[test]
    fn scans_grok_summaries_for_the_selected_project() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        let session_directory = roots.grok.join("encoded-project/grok-session");
        fs::create_dir_all(&session_directory).unwrap();
        fs::write(
            session_directory.join("summary.json"),
            serde_json::to_vec(&serde_json::json!({
                "info": {
                    "id": "grok-session",
                    "cwd": project,
                },
                "generated_title": "Implement Grok support",
                "session_summary": "fallback",
                "current_model_id": "grok-code-fast",
            }))
            .unwrap(),
        )
        .unwrap();

        let sessions =
            scan_agent_sessions_with_roots(SessionProvider::Grok, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "grok-session");
        assert_eq!(sessions[0].title, "Implement Grok support");
        assert_eq!(sessions[0].model.as_deref(), Some("grok-code-fast"));
        assert!(sessions[0].transcript_path.is_none());
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
            scan_agent_sessions_with_roots(SessionProvider::OhMyPi, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "session-1");
        assert_eq!(sessions[0].title, "Fix the sidebar");
        assert_eq!(
            sessions[0].transcript_path.as_deref(),
            Some(transcript.as_path())
        );
    }

    #[test]
    fn scans_and_combines_sessions_from_multiple_agents() {
        let temporary = tempfile::tempdir().unwrap();
        let roots = roots(temporary.path());
        let project = temporary.path().join("project");
        let codex_directory = roots.codex.join("sessions/2026/08/10");
        let omp_directory = roots.omp.join("encoded-project");
        fs::create_dir_all(&codex_directory).unwrap();
        fs::create_dir_all(&omp_directory).unwrap();
        fs::write(
            codex_directory.join("rollout.jsonl"),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-session\",\"cwd\":{:?}}}}}\n",
                project.to_string_lossy()
            ),
        )
        .unwrap();
        fs::write(
            omp_directory.join("session.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"omp-session\",\"cwd\":{:?}}}\n",
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let sessions = scan_agent_sessions_for_agents_with_roots(
            &[
                SessionProvider::OhMyPi,
                SessionProvider::Codex,
                SessionProvider::OhMyPi,
            ],
            &project,
            &roots,
            false,
        )
        .unwrap()
        .sessions;

        assert_eq!(sessions.len(), 2);
        assert!(
            sessions
                .iter()
                .any(|session| session.provider == SessionProvider::Codex)
        );
        assert!(
            sessions
                .iter()
                .any(|session| session.provider == SessionProvider::OhMyPi)
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
            scan_agent_sessions_with_roots(SessionProvider::OhMyPi, &project, &roots).unwrap();

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
            scan_agent_sessions_with_roots(SessionProvider::OhMyPi, &project, &roots).unwrap();

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
            scan_agent_sessions_with_roots(SessionProvider::Codex, &project, &roots).unwrap();

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
            SessionProvider::Pi,
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
            scan_agent_sessions_with_roots(SessionProvider::Claude, &project, &roots).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Session summary");
        assert_eq!(sessions[0].updated_at_ms, 42);
    }

    #[test]
    fn parses_opencode_json_output() {
        let project = Path::new("/workspace/project");
        let output = br#"[{"id":"ses_1","title":"Refactor auth","updated":99,"directory":"/workspace/project"}]"#;

        let sessions =
            parse_opencode_output(output, &mut ScanContext::new(project).unwrap()).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].provider, SessionProvider::OpenCode);
        assert_eq!(sessions[0].id, "ses_1");
        assert_eq!(sessions[0].title, "Refactor auth");
    }

    #[test]
    fn opencode_keeps_all_valid_entries_and_reports_a_damaged_entry() {
        let project = Path::new("/workspace/project");
        let mut entries = (0..201)
            .map(|n| {
                serde_json::json!({
                    "id": format!("session-{n}"), "title": "Conversation", "updated": n,
                    "directory": project,
                })
            })
            .collect::<Vec<_>>();
        entries.push(serde_json::json!({"id":"damaged"}));
        entries.push(serde_json::json!({
            "id":"other", "title":"Other project", "updated":999,
            "directory":"/workspace/project/../other",
        }));
        let mut context = ScanContext::new(project).unwrap();
        let sessions =
            parse_opencode_output(&serde_json::to_vec(&entries).unwrap(), &mut context).unwrap();
        assert_eq!(sessions.len(), 201);
        assert!(sessions.iter().any(|session| session.id == "session-0"));
        assert!(!sessions.iter().any(|session| session.id == "other"));
        assert_eq!(context.warnings.len(), 1);
    }

    #[test]
    fn opencode_empty_store_is_not_a_failed_scan_but_invalid_output_is() {
        let mut context = ScanContext::new(Path::new("/workspace/project")).unwrap();
        assert!(parse_opencode_output(b"", &mut context).unwrap().is_empty());
        assert!(context.warnings.is_empty());
        assert!(parse_opencode_output(b"not a session list", &mut context).is_err());
    }
}
