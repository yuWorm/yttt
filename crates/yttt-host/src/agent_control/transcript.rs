//! Reads an Agent's last reply from the transcript its provider writes.
//! Only the end of the file is read; formats this module does not know
//! yield no reply rather than a guess.

use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    path::Path,
};

use serde_json::Value;

/// How much of a transcript's end is searched for the last reply.
const TAIL_BYTES: u64 = 8 * 1024 * 1024;

/// The provider's last reply in `transcript`, if this module knows its format.
pub(crate) fn final_reply(provider: &str, transcript: &Path) -> Option<String> {
    let tail = read_tail(transcript)?;
    let entries = tail
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok());
    match provider {
        "claude" => claude_reply(entries),
        "codex" => codex_reply(entries),
        _ => None,
    }
}

fn read_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    // A tail that starts mid-line drops the partial first line.
    Some(if start == 0 {
        text
    } else {
        text.split_once('\n')
            .map_or_else(String::new, |(_, rest)| rest.to_string())
    })
}

/// Claude Code writes one entry per content block. The reply is the last
/// text block an assistant wrote after the latest prompt from a person.
fn claude_reply(entries: impl Iterator<Item = Value>) -> Option<String> {
    let mut reply = None;
    for entry in entries {
        let message = &entry["message"];
        match entry["type"].as_str() {
            Some("user") if is_prompt(&message["content"]) => reply = None,
            Some("assistant") => {
                if let Some(items) = message["content"].as_array() {
                    for item in items {
                        if item["type"] == "text"
                            && let Some(text) = item["text"].as_str()
                            && !text.trim().is_empty()
                        {
                            reply = Some(text.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    reply
}

/// A user entry is a person's prompt unless it only returns tool results.
fn is_prompt(content: &Value) -> bool {
    match content {
        Value::String(_) => true,
        Value::Array(items) => items.iter().any(|item| item["type"] == "text"),
        _ => false,
    }
}

/// Codex records each finished task with its last message; earlier
/// versions only have the assistant's response items.
fn codex_reply(entries: impl Iterator<Item = Value>) -> Option<String> {
    let mut completed = None;
    let mut message = None;
    for entry in entries {
        let payload = &entry["payload"];
        match (entry["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("task_started")) => {
                completed = None;
                message = None;
            }
            (Some("event_msg"), Some("task_complete")) => {
                completed = payload["last_agent_message"].as_str().map(str::to_owned);
            }
            (Some("response_item"), Some("message")) if payload["role"] == "assistant" => {
                let text = payload["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["type"] == "output_text")
                    .filter_map(|item| item["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    message = Some(text);
                }
            }
            _ => {}
        }
    }
    completed.filter(|text| !text.trim().is_empty()).or(message)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use serde_json::json;

    use super::*;

    fn transcript(entries: &[Value]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for entry in entries {
            writeln!(file, "{entry}").unwrap();
        }
        file
    }

    #[test]
    fn claude_reply_is_the_last_text_after_the_latest_prompt() {
        let file = transcript(&[
            json!({"type": "user", "message": {"role": "user", "content": "first task"}}),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "old answer"}]}}),
            json!({"type": "user", "message": {"role": "user", "content": "second task"}}),
            json!({"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "…"}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Reading the file"}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read"}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "data"}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Done: 3 fixes"}]}}),
            json!({"type": "system", "subtype": "stop"}),
        ]);
        assert_eq!(
            final_reply("claude", file.path()).as_deref(),
            Some("Done: 3 fixes")
        );

        let pending = transcript(&[
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "old answer"}]}}),
            json!({"type": "user", "message": {"content": [{"type": "text", "text": "new task"}]}}),
        ]);
        assert_eq!(final_reply("claude", pending.path()), None);
    }

    #[test]
    fn codex_reply_prefers_the_completed_tasks_last_message() {
        let file = transcript(&[
            json!({"type": "event_msg", "payload": {"type": "task_started"}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Working on it"}]}}),
            json!({"type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": "All tests pass"}}),
        ]);
        assert_eq!(
            final_reply("codex", file.path()).as_deref(),
            Some("All tests pass")
        );

        let older = transcript(&[
            json!({"type": "event_msg", "payload": {"type": "task_started"}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "task"}]}}),
            json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Patched"}]}}),
        ]);
        assert_eq!(
            final_reply("codex", older.path()).as_deref(),
            Some("Patched")
        );

        let restarted = transcript(&[
            json!({"type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": "previous"}}),
            json!({"type": "event_msg", "payload": {"type": "task_started"}}),
        ]);
        assert_eq!(final_reply("codex", restarted.path()), None);
    }

    #[test]
    fn unknown_providers_missing_files_and_cut_lines_yield_nothing_wrong() {
        let file = transcript(&[json!({"type": "assistant"})]);
        assert_eq!(final_reply("grok", file.path()), None);
        assert_eq!(
            final_reply("claude", Path::new("/nonexistent/transcript.jsonl")),
            None
        );

        let mut long = tempfile::NamedTempFile::new().unwrap();
        // The tail starts inside this padding line, which must be skipped.
        writeln!(long, "{}", "x".repeat(TAIL_BYTES as usize)).unwrap();
        writeln!(
            long,
            "{}",
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "kept"}]}})
        )
        .unwrap();
        assert_eq!(final_reply("claude", long.path()).as_deref(), Some("kept"));
    }
}
