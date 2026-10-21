use std::{
    fs,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

use serde_json::json;
use yttt_agent_providers::sessions::{
    AgentSessionRoots, SessionProvider, scan_agent_sessions_for_agents_with_roots,
};

fn roots(root: &Path) -> AgentSessionRoots {
    AgentSessionRoots {
        codex: root.join("codex"),
        claude: root.join("claude"),
        grok: root.join("grok"),
        pi: root.join("pi"),
        omp: root.join("omp"),
    }
}

const FILE_PROVIDERS: [SessionProvider; 5] = [
    SessionProvider::Codex,
    SessionProvider::Claude,
    SessionProvider::Grok,
    SessionProvider::Pi,
    SessionProvider::OhMyPi,
];

fn session_file(
    roots: &AgentSessionRoots,
    provider: SessionProvider,
    project: &Path,
    id: &str,
    modified: u64,
) -> std::path::PathBuf {
    let (path, value) = match provider {
        SessionProvider::Codex => (
            roots.codex.join(format!("sessions/{id}.jsonl")),
            json!({"type":"session_meta", "payload":{"id":id,"cwd":project}}),
        ),
        SessionProvider::Claude => (
            roots.claude.join(format!("projects/project/{id}.jsonl")),
            json!({"type":"user", "sessionId":id,"cwd":project,"message":{"role":"user","content":"Find this conversation"}}),
        ),
        SessionProvider::Grok => (
            roots.grok.join(format!("{id}/summary.json")),
            json!({"info":{"id":id,"cwd":project},"generated_title":"Find this conversation"}),
        ),
        SessionProvider::Pi | SessionProvider::OhMyPi => (
            if provider == SessionProvider::Pi {
                &roots.pi
            } else {
                &roots.omp
            }
            .join(format!("project/{id}.jsonl")),
            json!({"type":"session","id":id,"cwd":project}),
        ),
        SessionProvider::OpenCode => unreachable!("OpenCode uses its native session command"),
    };
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("{value}\n")).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(modified))
        .unwrap();
    path
}

#[test]
fn every_file_provider_keeps_old_sessions_and_combining_providers_does_not_hide_them() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    for provider in FILE_PROVIDERS {
        for n in 0..101 {
            session_file(&roots, provider, &project, &format!("session-{n}"), n + 1);
        }
        let sessions =
            scan_agent_sessions_for_agents_with_roots(&[provider], &project, &roots, false)
                .unwrap()
                .sessions;
        assert_eq!(
            sessions.len(),
            101,
            "{} must keep every matching conversation",
            provider.id()
        );
        assert!(sessions.iter().any(|session| session.id == "session-0"));
    }
    let sessions =
        scan_agent_sessions_for_agents_with_roots(&FILE_PROVIDERS, &project, &roots, false)
            .unwrap()
            .sessions;
    assert_eq!(
        sessions.len(),
        505,
        "one provider must not consume another provider's result quota"
    );
}

#[test]
fn unrelated_recent_files_cannot_hide_an_older_project_session() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    session_file(&roots, SessionProvider::Codex, &project, "wanted", 1);
    for n in 0..5_000 {
        session_file(
            &roots,
            SessionProvider::Codex,
            &temp.path().join("other"),
            &format!("other-{n}"),
            n + 2,
        );
    }
    let sessions = scan_agent_sessions_for_agents_with_roots(
        &[SessionProvider::Codex],
        &project,
        &roots,
        false,
    )
    .unwrap()
    .sessions;
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        ["wanted"]
    );
}

#[cfg(unix)]
#[test]
fn every_file_provider_matches_directory_aliases_without_including_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    let other = temp.path().join("other");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&other).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&project, &alias).unwrap();
    for provider in FILE_PROVIDERS {
        session_file(&roots, provider, &project, "wanted", 1);
        session_file(&roots, provider, &project.join("../other"), "sibling", 2);
        session_file(
            &roots,
            provider,
            &temp.path().join("project-extra"),
            "prefix-sibling",
            3,
        );
        let sessions =
            scan_agent_sessions_for_agents_with_roots(&[provider], &alias, &roots, false)
                .unwrap()
                .sessions;
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["wanted"],
            "{}",
            provider.id()
        );
    }
}

#[test]
fn damaged_claude_index_does_not_hide_its_transcripts_or_other_providers() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    for provider in FILE_PROVIDERS {
        session_file(&roots, provider, &project, "wanted", 1);
    }
    fs::write(
        roots.claude.join("projects/project/sessions-index.json"),
        "{",
    )
    .unwrap();
    let scan = scan_agent_sessions_for_agents_with_roots(&FILE_PROVIDERS, &project, &roots, false)
        .unwrap();
    assert_eq!(
        scan.sessions.len(),
        5,
        "a broken optional index must not discard valid histories"
    );
    assert_eq!(scan.warnings.len(), 1, "the damaged index must be reported");
}

#[test]
fn identity_after_large_preamble_is_still_discovered() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    let preamble = format!("{}\n", json!({"type":"snapshot","data":"x".repeat(2048)})).repeat(300);
    for provider in [
        SessionProvider::Codex,
        SessionProvider::Claude,
        SessionProvider::Pi,
        SessionProvider::OhMyPi,
    ] {
        let path = session_file(&roots, provider, &project, "wanted", 1);
        let header = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{preamble}{header}")).unwrap();
        let scan = scan_agent_sessions_for_agents_with_roots(&[provider], &project, &roots, false)
            .unwrap();
        assert_eq!(
            scan.sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["wanted"],
            "{}",
            provider.id()
        );
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
    }
    let path = session_file(&roots, SessionProvider::Grok, &project, "wanted", 1);
    let mut summary: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["extra"] = json!("x".repeat(600 * 1024));
    fs::write(path, serde_json::to_vec(&summary).unwrap()).unwrap();
    let scan = scan_agent_sessions_for_agents_with_roots(
        &[SessionProvider::Grok],
        &project,
        &roots,
        false,
    )
    .unwrap();
    assert_eq!(scan.sessions[0].id, "wanted");
    assert!(scan.warnings.is_empty());
}

#[test]
fn failed_provider_and_damaged_transcript_preserve_other_sessions_with_warnings() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    fs::create_dir_all(&roots.codex).unwrap();
    fs::write(roots.codex.join("sessions"), "not a directory").unwrap();
    let path = session_file(&roots, SessionProvider::Claude, &project, "wanted", 1);
    fs::write(path.parent().unwrap().join("broken.jsonl"), "not json\n").unwrap();
    let scan = scan_agent_sessions_for_agents_with_roots(
        &[SessionProvider::Codex, SessionProvider::Claude],
        &project,
        &roots,
        false,
    )
    .unwrap();
    assert_eq!(
        scan.sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        ["wanted"]
    );
    assert!(
        scan.warnings
            .iter()
            .any(|warning| warning.contains("codex"))
    );
    assert!(
        scan.warnings
            .iter()
            .any(|warning| warning.contains("broken.jsonl"))
    );
}

#[cfg(unix)]
#[test]
fn linked_history_directories_are_scanned_once_even_with_cycles() {
    let temp = tempfile::tempdir().unwrap();
    let external = roots(temp.path().join("external").as_path());
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    let transcript = session_file(&external, SessionProvider::OhMyPi, &project, "wanted", 1);
    fs::create_dir_all(&roots.omp).unwrap();
    std::os::unix::fs::symlink(&external.omp, roots.omp.join("linked")).unwrap();
    std::os::unix::fs::symlink(&roots.omp, transcript.parent().unwrap().join("cycle")).unwrap();
    let scan = scan_agent_sessions_for_agents_with_roots(
        &[SessionProvider::OhMyPi],
        &project,
        &roots,
        false,
    )
    .unwrap();
    assert_eq!(
        scan.sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        ["wanted"]
    );
    assert!(scan.warnings.is_empty());
}

#[test]
fn claude_keeps_the_newest_duplicate_index_entry_and_skips_only_damaged_entries() {
    let temp = tempfile::tempdir().unwrap();
    let roots = roots(temp.path());
    let project = temp.path().join("project");
    let index = roots.claude.join("projects/project/sessions-index.json");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    let entry = |modified, title| {
        json!({
            "sessionId":"wanted", "fullPath":project.join("wanted.jsonl"),
            "fileMtime":modified, "summary":title, "projectPath":project,
        })
    };
    fs::write(
        index,
        serde_json::to_vec(&json!({
            "entries":[entry(2,"Latest summary"), entry(1,"Old summary"), {"sessionId":"damaged"}],
        }))
        .unwrap(),
    )
    .unwrap();
    let scan = scan_agent_sessions_for_agents_with_roots(
        &[SessionProvider::Claude],
        &project,
        &roots,
        false,
    )
    .unwrap();
    assert_eq!(scan.sessions.len(), 1);
    assert_eq!(scan.sessions[0].id, "wanted");
    assert_eq!(scan.sessions[0].title, "Latest summary");
    assert_eq!(scan.sessions[0].updated_at_ms, 2);
    assert_eq!(scan.warnings.len(), 1);
}
