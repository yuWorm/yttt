use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom, Write as _},
    path::{Component, Path, PathBuf},
    process::Command,
};

use chrono::{SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::sessions::{AgentSessionRoots, SessionProvider};

mod native;

const MAX_TITLE_CHARS: usize = 200;
const MAX_SESSION_ID_CHARS: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionOperation {
    Rename { title: String },
    Delete,
}

/// Performs a permanent session mutation after resolving the session inside the
/// selected provider store. The public API deliberately takes an ID rather than
/// a path so a caller cannot redirect a deletion at an arbitrary file.
pub fn mutate_agent_session(
    provider: SessionProvider,
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    allow_native_commands: bool,
    operation: &SessionOperation,
) -> Result<(), String> {
    validate_session_id(session_id)?;
    validate_operation(operation)?;
    let project_path = canonical_project_path(project_path)?;

    match provider {
        SessionProvider::Codex => {
            if !allow_native_commands {
                return Err(
                    "codex session mutation needs its native interface, but native commands are disabled for this profile"
                        .to_string(),
                );
            }
            ensure_codex_target_is_not_open(session_id, &project_path, roots)?;
            native::mutate(provider, session_id, &project_path, roots, operation)
        }
        SessionProvider::OpenCode => {
            if !allow_native_commands {
                return Err(
                    "opencode session mutation needs its native interface, but native commands are disabled for this profile"
                        .to_string(),
                );
            }
            ensure_opencode_store_is_not_open(roots)?;
            native::mutate(provider, session_id, &project_path, roots, operation)
        }
        SessionProvider::Claude => mutate_claude(session_id, &project_path, roots, operation),
        SessionProvider::Grok => mutate_grok(
            session_id,
            &project_path,
            roots,
            allow_native_commands,
            operation,
        ),
        SessionProvider::Pi => mutate_pi(session_id, &project_path, roots, operation, false),
        SessionProvider::OhMyPi => mutate_pi(session_id, &project_path, roots, operation, true),
    }
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.trim().is_empty()
        || session_id.chars().count() > MAX_SESSION_ID_CHARS
        || session_id.contains(['/', '\\'])
        || session_id == "."
        || session_id == ".."
        || session_id.chars().any(char::is_control)
    {
        return Err("invalid session ID".to_string());
    }
    Ok(())
}

fn validate_operation(operation: &SessionOperation) -> Result<(), String> {
    let SessionOperation::Rename { title } = operation else {
        return Ok(());
    };
    if title.trim().is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title.chars().any(char::is_control)
    {
        return Err("a session title must be nonblank, contain no control characters, and be at most 200 characters".to_string());
    }
    Ok(())
}

fn canonical_project_path(project_path: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(project_path).map_err(|error| {
        format!(
            "cannot resolve selected project {}: {error}",
            project_path.display()
        )
    })?;
    if !path.is_dir() {
        return Err(format!(
            "selected project {} is not a directory",
            path.display()
        ));
    }
    Ok(path)
}

fn ensure_codex_target_is_not_open(
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
) -> Result<(), String> {
    let store = canonical_store_root(&roots.codex, "Codex")?;
    let sessions = store.join("sessions");
    let session_metadata = fs::symlink_metadata(&sessions).map_err(|error| {
        format!(
            "cannot verify whether Codex session {session_id} is active: could not inspect {}: {error}",
            sessions.display()
        )
    })?;
    if session_metadata.file_type().is_symlink() || !session_metadata.is_dir() {
        return Err(format!(
            "cannot verify whether Codex session {session_id} is active: {} is not a safe session directory",
            sessions.display()
        ));
    }

    let mut paths = Vec::new();
    let mut found_identity = false;
    for path in regular_files(&sessions)? {
        if path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        {
            continue;
        }
        if codex_transcript_matches(&path, session_id, project_path)? {
            found_identity = true;
            paths.push(path);
        }
    }
    let index = store.join("session_index.jsonl");
    if index.exists() {
        ensure_regular_file(&index)?;
        if codex_index_contains(&index, session_id)? {
            found_identity = true;
        }
        // This append-only index is shared by Codex sessions in this store,
        // so an external writer makes the native mutation unsafe even when it
        // is currently writing another session.
        paths.push(index);
    }
    if !found_identity {
        return Err(format!(
            "cannot verify whether Codex session {session_id} is active because no matching local session artifact was found"
        ));
    }
    for path in paths {
        ensure_not_open(&path)?;
    }
    Ok(())
}

fn codex_transcript_matches(
    path: &Path,
    session_id: &str,
    project_path: &Path,
) -> Result<bool, String> {
    let file = File::open(path).map_err(|error| {
        format!(
            "could not inspect Codex session history {}: {error}",
            path.display()
        )
    })?;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| {
            format!(
                "could not read Codex session history {}: {error}",
                path.display()
            )
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            // A damaged unrelated transcript does not identify the requested
            // session and must not prevent a safe mutation of another one.
            continue;
        };
        let payload = value.get("payload");
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            return Ok(payload
                .and_then(|payload| payload.get("id").or_else(|| payload.get("session_id")))
                .and_then(Value::as_str)
                == Some(session_id)
                && payload
                    .and_then(|payload| payload.get("cwd"))
                    .and_then(Value::as_str)
                    .is_some_and(|cwd| belongs_to_project(Path::new(cwd), project_path)));
        }
    }
    Ok(false)
}

fn codex_index_contains(path: &Path, session_id: &str) -> Result<bool, String> {
    let file = File::open(path).map_err(|error| {
        format!(
            "could not inspect Codex session index {}: {error}",
            path.display()
        )
    })?;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| {
            format!(
                "could not read Codex session index {}: {error}",
                path.display()
            )
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id").and_then(Value::as_str) == Some(session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_opencode_store_is_not_open(roots: &AgentSessionRoots) -> Result<(), String> {
    // OpenCode selects its database from its native profile. Its native adapter
    // rejects isolated roots, so never inspect an unrelated user's store for a
    // request that cannot proceed.
    let native_roots = AgentSessionRoots::native().ok_or_else(|| {
        "OpenCode native mutation needs a host profile with a configured home directory".to_string()
    })?;
    if roots.codex != native_roots.codex
        || roots.claude != native_roots.claude
        || roots.grok != native_roots.grok
        || roots.pi != native_roots.pi
        || roots.omp != native_roots.omp
    {
        return Ok(());
    }

    let output = Command::new("opencode")
        .args(["--pure", "debug", "paths"])
        .output()
        .map_err(|error| format!("cannot discover the OpenCode session store: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot discover the OpenCode session store: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let data = stdout
        .lines()
        .find_map(|line| line.strip_prefix("data").map(str::trim))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "OpenCode did not report its data directory".to_string())?;
    for path in [
        data.join("opencode.db"),
        data.join("opencode.db-wal"),
        data.join("opencode.db-shm"),
    ] {
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "cannot inspect OpenCode session store artifact {}: {error}",
                    path.display()
                ));
            }
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(format!(
                    "refusing to inspect non-regular OpenCode session store artifact {}",
                    path.display()
                ));
            }
            Ok(_) => ensure_not_open(&path)?,
        }
    }
    Ok(())
}

fn mutate_claude(
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    operation: &SessionOperation,
) -> Result<(), String> {
    let store = canonical_store_root(&roots.claude.join("projects"), "Claude")?;
    let target = find_claude_target(&store, session_id, project_path)?;
    let mut paths = target.transcripts.clone();
    paths.extend(target.indexes.iter().cloned());
    let sidecars = if matches!(operation, SessionOperation::Delete) {
        target
            .transcripts
            .iter()
            .filter_map(|transcript| claude_session_sidecar(transcript, &store).transpose())
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    for sidecar in &sidecars {
        paths.extend(regular_files(sidecar)?);
    }
    let _locks = lock_paths(&paths)?;

    match operation {
        SessionOperation::Rename { title } => {
            for transcript in &target.transcripts {
                append_json_line(
                    transcript,
                    &json!({
                        "type": "custom-title",
                        "sessionId": session_id,
                        "customTitle": title,
                    }),
                )?;
            }
            for index in &target.indexes {
                update_claude_index(index, session_id, project_path, Some(title))?;
            }
        }
        SessionOperation::Delete => {
            // Remove indexes first so a failed transcript removal cannot leave
            // an apparently live entry pointing at an already deleted file.
            for index in &target.indexes {
                update_claude_index(index, session_id, project_path, None)?;
            }
            for transcript in &target.transcripts {
                fs::remove_file(transcript).map_err(|error| {
                    format!(
                        "could not delete Claude transcript {}: {error}",
                        transcript.display()
                    )
                })?;
            }
            for sidecar in sidecars {
                fs::remove_dir_all(&sidecar).map_err(|error| {
                    format!(
                        "could not delete Claude session sidecar {}: {error}",
                        sidecar.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct ClaudeTarget {
    transcripts: Vec<PathBuf>,
    indexes: Vec<PathBuf>,
}

fn claude_session_sidecar(transcript: &Path, store: &Path) -> Result<Option<PathBuf>, String> {
    let directory = transcript.with_extension("");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "could not inspect Claude session sidecar {}: {error}",
            directory.display()
        )),
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to follow Claude session sidecar symlink {}",
            directory.display()
        )),
        Ok(metadata) if metadata.is_dir() => {
            let directory = fs::canonicalize(&directory).map_err(|error| {
                format!(
                    "could not resolve Claude session sidecar {}: {error}",
                    directory.display()
                )
            })?;
            if directory == store || !directory.starts_with(store) {
                return Err(format!(
                    "Claude sidecar {} escapes its configured store",
                    directory.display()
                ));
            }
            Ok(Some(directory))
        }
        Ok(_) => Err(format!(
            "Claude session sidecar {} is not a directory",
            directory.display()
        )),
    }
}

fn find_claude_target(
    store: &Path,
    session_id: &str,
    project_path: &Path,
) -> Result<ClaudeTarget, String> {
    let mut target = ClaudeTarget::default();
    for path in regular_files(store)? {
        // Child transcripts belong to the parent sidecar, not the selectable
        // session namespace. Their contents must not block parent management.
        if path.strip_prefix(store).is_ok_and(|relative| {
            relative
                .components()
                .any(|component| component.as_os_str() == "subagents")
        }) {
            continue;
        }
        match path.file_name().and_then(|name| name.to_str()) {
            Some("sessions-index.json") => {
                if claude_index_has_session(&path, session_id, project_path)? {
                    target.indexes.push(path);
                }
            }
            _ if path
                .extension()
                .is_some_and(|extension| extension == "jsonl") =>
            {
                if claude_transcript_matches(&path, session_id, project_path)? {
                    target.transcripts.push(path);
                }
            }
            _ => {}
        }
    }
    if target.transcripts.is_empty() && target.indexes.is_empty() {
        return Err("Claude session was not found in the selected project store".to_string());
    }
    Ok(target)
}

fn claude_transcript_matches(
    path: &Path,
    session_id: &str,
    project_path: &Path,
) -> Result<bool, String> {
    let records = read_jsonl(path)?;
    if records
        .iter()
        .any(|record| record.value.get("isSidechain").and_then(Value::as_bool) == Some(true))
    {
        return Ok(false);
    }
    let id = records.iter().find_map(|record| {
        record
            .value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    });
    let cwd = records.iter().find_map(|record| {
        record
            .value
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
    });
    Ok(id.as_deref() == Some(session_id)
        && cwd
            .as_deref()
            .is_some_and(|cwd| belongs_to_project(cwd, project_path)))
}

fn claude_index_has_session(
    path: &Path,
    session_id: &str,
    project_path: &Path,
) -> Result<bool, String> {
    let index = read_json(path)?;
    let entries = index
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Claude index {} has no entries array", path.display()))?;
    Ok(entries.iter().any(|entry| {
        entry.get("sessionId").and_then(Value::as_str) == Some(session_id)
            && entry
                .get("projectPath")
                .and_then(Value::as_str)
                .is_some_and(|cwd| belongs_to_project(Path::new(cwd), project_path))
            && entry.get("isSidechain").and_then(Value::as_bool) != Some(true)
    }))
}

fn update_claude_index(
    path: &Path,
    session_id: &str,
    project_path: &Path,
    title: Option<&str>,
) -> Result<(), String> {
    let mut index = read_json(path)?;
    let entries = index
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("Claude index {} has no entries array", path.display()))?;
    let mut changed = false;
    if let Some(title) = title {
        for entry in entries.iter_mut() {
            if claude_index_entry_matches(entry, session_id, project_path) {
                entry["summary"] = Value::String(title.to_string());
                changed = true;
            }
        }
    } else {
        let before = entries.len();
        entries.retain(|entry| !claude_index_entry_matches(entry, session_id, project_path));
        changed = entries.len() != before;
    }
    if changed {
        write_json_atomic(path, &index)?;
    }
    Ok(())
}

fn claude_index_entry_matches(entry: &Value, session_id: &str, project_path: &Path) -> bool {
    entry.get("sessionId").and_then(Value::as_str) == Some(session_id)
        && entry
            .get("projectPath")
            .and_then(Value::as_str)
            .is_some_and(|cwd| belongs_to_project(Path::new(cwd), project_path))
        && entry.get("isSidechain").and_then(Value::as_bool) != Some(true)
}

fn mutate_pi(
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    operation: &SessionOperation,
    omp: bool,
) -> Result<(), String> {
    let provider = if omp { "OMP" } else { "Pi" };
    let root = if omp { &roots.omp } else { &roots.pi };
    let store = canonical_store_root(root, provider)?;
    let target = find_pi_target(&store, session_id, project_path, provider)?;
    let mut paths = vec![target.transcript.clone()];
    let mut companion = None;
    let mut lock_path = None;
    if omp {
        companion = omp_companion_dir(&target.transcript, &store)?;
        if let Some(directory) = &companion {
            paths.extend(regular_files(directory)?);
        }
        lock_path = omp_lock_path(&target.transcript)?;
        if let Some(path) = &lock_path {
            paths.push(path.clone());
        }
    }
    let _locks = lock_paths(&paths)?;

    match operation {
        SessionOperation::Rename { title } if omp => {
            rename_omp(
                &target.transcript,
                &target.records,
                target.last_record_id.as_deref(),
                title,
            )?;
        }
        SessionOperation::Rename { title } => {
            append_json_line(
                &target.transcript,
                &json!({
                    "type": "session_info",
                    "id": new_record_id(),
                    "parentId": target.last_record_id,
                    "timestamp": timestamp(),
                    "name": title,
                }),
            )?;
        }
        SessionOperation::Delete => {
            fs::remove_file(&target.transcript).map_err(|error| {
                format!(
                    "could not delete {provider} transcript {}: {error}",
                    target.transcript.display()
                )
            })?;
            if let Some(directory) = companion {
                fs::remove_dir_all(&directory).map_err(|error| {
                    format!(
                        "could not delete OMP session sidecar {}: {error}",
                        directory.display()
                    )
                })?;
            }
            if let Some(path) = lock_path {
                // This exact lock belongs to the transcript and was verified
                // regular before locking. It is no longer useful after delete.
                fs::remove_file(&path).map_err(|error| {
                    format!("could not delete OMP lock {}: {error}", path.display())
                })?;
            }
        }
    }
    Ok(())
}

struct PiTarget {
    transcript: PathBuf,
    records: Vec<JsonlRecord>,
    last_record_id: Option<String>,
}

fn find_pi_target(
    store: &Path,
    session_id: &str,
    project_path: &Path,
    provider: &str,
) -> Result<PiTarget, String> {
    let mut matches = Vec::new();
    for path in regular_files(store)? {
        if path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        {
            continue;
        }
        let records = read_jsonl(&path)?;
        let header = records
            .iter()
            .find(|record| record.value.get("type").and_then(Value::as_str) == Some("session"));
        let Some(header) = header else {
            continue;
        };
        let version = header.value.get("version").and_then(Value::as_u64);
        if header.value.get("id").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        let Some(cwd) = header.value.get("cwd").and_then(Value::as_str) else {
            return Err(format!(
                "{provider} session {session_id} has no working directory"
            ));
        };
        if !belongs_to_project(Path::new(cwd), project_path) {
            continue;
        }
        if version != Some(3) {
            return Err(format!(
                "{provider} session {session_id} uses unsupported transcript version {}; only version 3 can be mutated safely",
                version.map_or_else(|| "missing".to_string(), |version| version.to_string())
            ));
        }
        let last_record_id = records.iter().rev().find_map(|record| {
            record
                .value
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });
        matches.push(PiTarget {
            transcript: path,
            records,
            last_record_id,
        });
    }
    match matches.len() {
        0 => Err(format!(
            "{provider} session was not found in the selected project store"
        )),
        1 => Ok(matches.pop().expect("one match")),
        _ => Err(format!(
            "{provider} session ID {session_id} is ambiguous in the selected project store"
        )),
    }
}

fn omp_companion_dir(transcript: &Path, store: &Path) -> Result<Option<PathBuf>, String> {
    let directory = transcript.with_extension("");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "could not inspect OMP sidecar {}: {error}",
            directory.display()
        )),
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to follow OMP session sidecar symlink {}",
            directory.display()
        )),
        Ok(metadata) if metadata.is_dir() => {
            let canonical = fs::canonicalize(&directory).map_err(|error| {
                format!(
                    "could not resolve OMP sidecar {}: {error}",
                    directory.display()
                )
            })?;
            if !canonical.starts_with(store) {
                return Err(format!(
                    "OMP sidecar {} escapes its configured store",
                    directory.display()
                ));
            }
            Ok(Some(canonical))
        }
        Ok(_) => Err(format!(
            "OMP sidecar {} is not a directory",
            directory.display()
        )),
    }
}

fn omp_lock_path(transcript: &Path) -> Result<Option<PathBuf>, String> {
    let file_name = transcript
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid OMP transcript path {}", transcript.display()))?;
    let path = transcript.with_file_name(format!(".{file_name}.lock.os"));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "could not inspect OMP lock {}: {error}",
            path.display()
        )),
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to follow OMP lock symlink {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_file() => Ok(Some(path)),
        Ok(_) => Err(format!("OMP lock {} is not a regular file", path.display())),
    }
}

fn rename_omp(
    path: &Path,
    records: &[JsonlRecord],
    last_record_id: Option<&str>,
    title: &str,
) -> Result<(), String> {
    const TITLE_SLOT_BYTES: usize = 256;

    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "could not read OMP session history {}: {error}",
            path.display()
        )
    })?;
    let lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let first = lines
        .first()
        .ok_or_else(|| format!("OMP session {} is empty", path.display()))?;
    let slot = parse_omp_title_slot(first, TITLE_SLOT_BYTES)?;
    let header = records
        .iter()
        .find(|record| record.value.get("type").and_then(Value::as_str) == Some("session"))
        .ok_or_else(|| format!("OMP session {} has no header", path.display()))?;
    let expected_header_line = usize::from(slot.is_some());
    if header.line != expected_header_line
        || header.value.get("version").and_then(Value::as_u64) != Some(3)
    {
        return Err(format!(
            "OMP session {} has an unsupported header/title-slot layout",
            path.display()
        ));
    }

    let previous_title = records
        .iter()
        .rev()
        .find_map(|record| {
            (record.value.get("type").and_then(Value::as_str) == Some("title_change"))
                .then(|| record.value.get("title").and_then(Value::as_str))
                .flatten()
                .and_then(clean_stored_title)
        })
        .or_else(|| {
            slot.as_ref()
                .and_then(|slot| clean_stored_title(&slot.title))
        })
        .or_else(|| {
            header
                .value
                .get("title")
                .and_then(Value::as_str)
                .and_then(clean_stored_title)
        });
    let updated_at = timestamp();
    let title_slot = serialize_omp_title_slot(title, &updated_at, TITLE_SLOT_BYTES)?;
    let mut updated_header = header.value.clone();
    updated_header["title"] = Value::String(title.to_string());
    updated_header["titleSource"] = Value::String("user".to_string());
    let updated_header = serde_json::to_string(&updated_header)
        .map_err(|error| format!("could not encode OMP session header: {error}"))?;

    let mut rewritten = String::with_capacity(source.len() + title.len() + TITLE_SLOT_BYTES);
    if slot.is_none() {
        rewritten.push_str(&title_slot);
    }
    for (line, raw) in lines.into_iter().enumerate() {
        if slot.is_some() && line == 0 {
            rewritten.push_str(&title_slot);
        } else if line == header.line {
            rewritten.push_str(&updated_header);
            if raw.ends_with("\r\n") {
                rewritten.push_str("\r\n");
            } else if raw.ends_with('\n') {
                rewritten.push('\n');
            } else {
                rewritten.push('\n');
            }
        } else {
            rewritten.push_str(raw);
        }
    }
    if !rewritten.ends_with('\n') {
        rewritten.push('\n');
    }
    let mut title_change = json!({
        "type": "title_change",
        "id": new_omp_entry_id(),
        "parentId": last_record_id,
        "timestamp": updated_at,
        "title": title,
        "source": "user",
    });
    if let Some(previous_title) = previous_title {
        title_change["previousTitle"] = Value::String(previous_title);
    }
    let title_change = serde_json::to_string(&title_change)
        .map_err(|error| format!("could not encode OMP title change: {error}"))?;
    rewritten.push_str(&title_change);
    rewritten.push('\n');
    write_text_atomic(path, rewritten.as_bytes())
}

struct OmpTitleSlot {
    title: String,
}

fn parse_omp_title_slot(line: &str, expected_bytes: usize) -> Result<Option<OmpTitleSlot>, String> {
    if !line.ends_with('\n') {
        return Ok(None);
    }
    let value = line.trim_end_matches(['\r', '\n']);
    let Ok(value) = serde_json::from_str::<Value>(value) else {
        return Ok(None);
    };
    if value.get("type").and_then(Value::as_str) != Some("title") {
        return Ok(None);
    }
    if line.as_bytes().len() != expected_bytes
        || value.get("v").and_then(Value::as_u64) != Some(1)
        || value.get("title").and_then(Value::as_str).is_none()
        || value.get("updatedAt").and_then(Value::as_str).is_none()
        || value.get("pad").and_then(Value::as_str).is_none()
        || !matches!(
            value.get("source").and_then(Value::as_str),
            None | Some("auto" | "user")
        )
    {
        return Err("OMP session has an unsupported fixed title slot".to_string());
    }
    Ok(Some(OmpTitleSlot {
        title: value
            .get("title")
            .and_then(Value::as_str)
            .expect("checked title")
            .to_string(),
    }))
}

fn serialize_omp_title_slot(
    title: &str,
    updated_at: &str,
    target_bytes: usize,
) -> Result<String, String> {
    let unpadded = omp_title_slot_line(title, updated_at, "")?;
    let padding = target_bytes.checked_sub(unpadded.len()).ok_or_else(|| {
        "this title is too long for OMP's 256-byte title slot; use a shorter name".to_string()
    })?;
    omp_title_slot_line(title, updated_at, &" ".repeat(padding))
}

fn omp_title_slot_line(title: &str, updated_at: &str, pad: &str) -> Result<String, String> {
    let value = json!({
        "type": "title",
        "v": 1,
        "title": title,
        "source": "user",
        "updatedAt": updated_at,
        "pad": pad,
    });
    serde_json::to_string(&value)
        .map(|line| format!("{line}\n"))
        .map_err(|error| format!("could not encode OMP title slot: {error}"))
}

fn clean_stored_title(value: impl AsRef<str>) -> Option<String> {
    let title = value.as_ref().trim();
    (!title.is_empty()).then(|| title.to_string())
}

fn new_omp_entry_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn mutate_grok(
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    allow_native_commands: bool,
    operation: &SessionOperation,
) -> Result<(), String> {
    let store = canonical_store_root(&roots.grok, "Grok")?;
    let target = find_grok_target(&store, session_id, project_path)?;
    let paths = regular_files(&target.directory)?;
    let _locks = lock_paths(&paths)?;

    match operation {
        SessionOperation::Rename { title } => {
            if title.chars().count() > 100
                || title.chars().any(|c| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
            {
                return Err("Grok session titles allow at most 100 characters and no directional formatting controls".into());
            }
            let mut summary = read_json(&target.summary)?;
            let object = summary.as_object_mut().ok_or_else(|| {
                format!("Grok summary {} is not an object", target.summary.display())
            })?;
            // Grok's generated title doubles as its user-visible title. Marking
            // it manual prevents a later generated-summary refresh replacing it.
            object.insert(
                "generated_title".to_string(),
                Value::String(title.to_string()),
            );
            object.insert("title_is_manual".to_string(), Value::Bool(true));
            object.insert("updated_at".to_string(), Value::String(timestamp()));
            write_json_atomic(&target.summary, &summary)?;
        }
        SessionOperation::Delete
            if can_use_native_grok_store(&store, roots, allow_native_commands) =>
        {
            let output = Command::new("grok")
                .args(["sessions", "delete", session_id])
                .current_dir(project_path)
                .output()
                .map_err(|error| format!("could not run Grok session deletion: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "Grok session deletion failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            if target.summary.exists() {
                return Err(
                    "Grok acknowledged deletion but the selected session is still present"
                        .to_string(),
                );
            }
        }
        SessionOperation::Delete => {
            fs::remove_dir_all(&target.directory).map_err(|error| {
                format!(
                    "could not delete Grok session directory {}: {error}",
                    target.directory.display()
                )
            })?;
        }
    }
    Ok(())
}

struct GrokTarget {
    summary: PathBuf,
    directory: PathBuf,
}

fn find_grok_target(
    store: &Path,
    session_id: &str,
    project_path: &Path,
) -> Result<GrokTarget, String> {
    let mut matches = Vec::new();
    for path in regular_files(store)? {
        if path.file_name().and_then(|name| name.to_str()) != Some("summary.json") {
            continue;
        }
        let summary = read_json(&path)?;
        let Some(info) = summary.get("info") else {
            continue;
        };
        if info.get("id").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        let Some(cwd) = info.get("cwd").and_then(Value::as_str) else {
            return Err(format!(
                "Grok session {session_id} has no working directory"
            ));
        };
        if !belongs_to_project(Path::new(cwd), project_path) {
            continue;
        }
        let directory = path
            .parent()
            .ok_or_else(|| {
                format!(
                    "Grok summary {} has no containing directory",
                    path.display()
                )
            })?
            .to_path_buf();
        if directory == store {
            return Err("refusing to delete the configured Grok store itself".to_string());
        }
        matches.push(GrokTarget {
            summary: path,
            directory,
        });
    }
    match matches.len() {
        0 => Err("Grok session was not found in the selected project store".to_string()),
        1 => Ok(matches.pop().expect("one match")),
        _ => Err(format!(
            "Grok session ID {session_id} is ambiguous in the selected project store"
        )),
    }
}

fn can_use_native_grok_store(
    store: &Path,
    roots: &AgentSessionRoots,
    allow_native_commands: bool,
) -> bool {
    if !allow_native_commands {
        return false;
    }
    let Some(native_roots) = AgentSessionRoots::native() else {
        return false;
    };
    let Ok(native_store) = canonical_store_root(&native_roots.grok, "Grok") else {
        return false;
    };
    let configured_store = canonical_store_root(&roots.grok, "Grok");
    configured_store.is_ok_and(|configured| configured == store && configured == native_store)
}

#[derive(Clone)]
struct JsonlRecord {
    line: usize,
    value: Value,
}

fn read_jsonl(path: &Path) -> Result<Vec<JsonlRecord>, String> {
    let file = File::open(path)
        .map_err(|error| format!("could not open session history {}: {error}", path.display()))?;
    let mut records = Vec::new();
    for (line, result) in BufReader::new(file).lines().enumerate() {
        let source = result.map_err(|error| {
            format!("could not read session history {}: {error}", path.display())
        })?;
        if source.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(&source).map_err(|error| {
            format!(
                "invalid JSON in session history {} line {}: {error}",
                path.display(),
                line + 1
            )
        })?;
        records.push(JsonlRecord { line, value });
    }
    Ok(records)
}

fn append_json_line(path: &Path, value: &Value) -> Result<(), String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| format!("could not encode session record: {error}"))?;
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            format!(
                "could not open session history {} for update: {error}",
                path.display()
            )
        })?;
    if file
        .metadata()
        .map_err(|error| {
            format!(
                "could not inspect session history {}: {error}",
                path.display()
            )
        })?
        .len()
        > 0
    {
        file.seek(SeekFrom::End(-1)).map_err(|error| {
            format!("could not read session history {}: {error}", path.display())
        })?;
        let mut final_byte = [0];
        file.read_exact(&mut final_byte).map_err(|error| {
            format!("could not read session history {}: {error}", path.display())
        })?;
        file.seek(SeekFrom::End(0)).map_err(|error| {
            format!(
                "could not append session history {}: {error}",
                path.display()
            )
        })?;
        if final_byte != [b'\n'] {
            file.write_all(b"\n").map_err(|error| {
                format!(
                    "could not separate session history records in {}: {error}",
                    path.display()
                )
            })?;
        }
    }
    file.write_all(&encoded)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "could not append session history {}: {error}",
                path.display()
            )
        })
}

fn read_json(path: &Path) -> Result<Value, String> {
    let file =
        File::open(path).map_err(|error| format!("could not open {}: {error}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<(), String> {
    let source = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("could not encode {}: {error}", path.display()))?;
    write_text_atomic(path, &source)
}

fn write_text_atomic(path: &Path, source: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("session path {} has no parent", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "could not create temporary session update in {}: {error}",
            parent.display()
        )
    })?;
    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| {
                format!(
                    "could not preserve permissions for {}: {error}",
                    path.display()
                )
            })?;
    }
    temporary
        .write_all(source)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| {
            format!(
                "could not write session update for {}: {error}",
                path.display()
            )
        })?;
    temporary.persist(path).map_err(|error| {
        format!(
            "could not replace session file {}: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

struct FileLocks {
    _files: Vec<File>,
}

fn lock_paths(paths: &[PathBuf]) -> Result<FileLocks, String> {
    let mut unique = paths.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut locks = Vec::with_capacity(unique.len());
    for path in unique {
        ensure_regular_file(&path)?;
        ensure_not_open(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "could not open session artifact {}: {error}",
                    path.display()
                )
            })?;
        FileExt::try_lock_exclusive(&file).map_err(|error| {
            format!(
                "session artifact {} is busy and cannot be changed safely: {error}",
                path.display()
            )
        })?;
        locks.push(file);
    }
    Ok(FileLocks { _files: locks })
}

fn ensure_not_open(path: &Path) -> Result<(), String> {
    let output = Command::new("lsof")
        .args(["-nP", "-Fp"])
        .arg(path)
        .output()
        .map_err(|error| {
            format!(
                "cannot verify whether {} is active: {error}",
                path.display()
            )
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.lines().any(|line| line.starts_with('p')) {
        return Err(format!(
            "session artifact {} is open by another process; close the provider before changing this session",
            path.display()
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && !(output.status.code() == Some(1) && stderr.trim().is_empty()) {
        return Err(format!(
            "cannot verify whether {} is active: {}",
            path.display(),
            stderr.trim()
        ));
    }
    Ok(())
}

fn canonical_store_root(root: &Path, provider: &str) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        format!(
            "could not inspect {provider} session store {}: {error}",
            root.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to mutate through symbolic-link {provider} session store {}",
            root.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "{provider} session store {} is not a directory",
            root.display()
        ));
    }
    fs::canonicalize(root).map_err(|error| {
        format!(
            "could not resolve {provider} session store {}: {error}",
            root.display()
        )
    })
}

fn regular_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    while let Some(directory) = directories.pop() {
        let canonical = fs::canonicalize(&directory).map_err(|error| {
            format!(
                "could not resolve session directory {}: {error}",
                directory.display()
            )
        })?;
        if !canonical.starts_with(root) {
            return Err(format!(
                "session directory {} escapes its configured store",
                directory.display()
            ));
        }
        if !visited.insert(canonical.clone()) {
            continue;
        }
        for entry in fs::read_dir(&canonical).map_err(|error| {
            format!(
                "could not read session directory {}: {error}",
                canonical.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "could not read session directory {}: {error}",
                    canonical.display()
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!(
                    "could not inspect session artifact {}: {error}",
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn ensure_regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "could not inspect session artifact {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "refusing to mutate non-regular session artifact {}",
            path.display()
        ));
    }
    Ok(())
}

fn belongs_to_project(path: &Path, project_path: &Path) -> bool {
    path.is_absolute() && normalized_path(path).starts_with(project_path)
}

fn normalized_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
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

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn new_record_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
