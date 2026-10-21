use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;
use yttt_agent_providers::{
    session_management::{SessionOperation, mutate_agent_session},
    sessions::{AgentSessionRoots, SessionProvider, scan_agent_sessions_for_agents_with_roots},
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

fn project(root: &Path) -> PathBuf {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    project
}

fn session_title(
    provider: SessionProvider,
    project: &Path,
    roots: &AgentSessionRoots,
    session_id: &str,
) -> String {
    scan_agent_sessions_for_agents_with_roots(&[provider], project, roots, false)
        .unwrap()
        .sessions
        .into_iter()
        .find(|session| session.id == session_id)
        .unwrap()
        .title
}

fn claude_session(roots: &AgentSessionRoots, project: &Path, id: &str, summary: &str) -> PathBuf {
    let directory = roots.claude.join("projects/encoded");
    fs::create_dir_all(&directory).unwrap();
    let transcript = directory.join(format!("{id}.jsonl"));
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({
                "type": "user", "sessionId": id, "cwd": project,
                "message": {"role": "user", "content": "fallback prompt"},
            })
        ),
    )
    .unwrap();
    let index = directory.join("sessions-index.json");
    let mut entries = if index.exists() {
        serde_json::from_slice::<serde_json::Value>(&fs::read(&index).unwrap()).unwrap()["entries"]
            .as_array()
            .unwrap()
            .clone()
    } else {
        Vec::new()
    };
    entries.push(json!({
        "sessionId": id,
        "fullPath": transcript,
        "fileMtime": 1,
        "firstPrompt": "fallback prompt",
        "summary": summary,
        "projectPath": project,
        "isSidechain": false,
    }));
    fs::write(
        &index,
        serde_json::to_vec(&json!({"entries": entries})).unwrap(),
    )
    .unwrap();
    transcript
}

fn pi_session(root: &Path, project: &Path, id: &str, name: &str) -> PathBuf {
    fs::create_dir_all(root.join("encoded")).unwrap();
    let path = root.join(format!("encoded/{id}.jsonl"));
    fs::write(
        &path,
        format!(
            "{}\n{}\n{}\n",
            json!({"type":"session","version":3,"id":id,"cwd":project}),
            json!({"type":"session_info","id":"old","name":"old name"}),
            json!({"type":"session_info","id":"current","name":name}),
        ),
    )
    .unwrap();
    path
}

fn omp_session(root: &Path, project: &Path, id: &str) -> PathBuf {
    fs::create_dir_all(root.join("encoded")).unwrap();
    let path = root.join(format!("encoded/{id}.jsonl"));
    let mut slot = json!({"type":"title","v":1,"title":"slot title","source":"user","updatedAt":"2026-10-21T08:33:44.146Z","pad":""});
    let padding = 256 - (serde_json::to_vec(&slot).unwrap().len() + 1);
    slot["pad"] = json!(" ".repeat(padding));
    fs::write(
        &path,
        format!(
            "{}\n{}\n{}\n{}\n",
            slot,
            json!({"type":"session","version":3,"id":id,"cwd":project,"title":"header title"}),
            json!({"type":"title_change","title":"older title change"}),
            json!({"type":"message","message":{"role":"user","content":"fallback prompt"}}),
        ),
    )
    .unwrap();
    path
}

fn grok_session(root: &Path, project: &Path, id: &str, title: &str) -> PathBuf {
    let directory = root.join(id);
    fs::create_dir_all(&directory).unwrap();
    let summary = directory.join("summary.json");
    fs::write(
        &summary,
        serde_json::to_vec(&json!({
            "info": {"id": id, "cwd": project},
            "generated_title": title,
            "session_summary": "generated fallback",
        }))
        .unwrap(),
    )
    .unwrap();
    summary
}

#[test]
fn claude_rename_round_trips_full_latest_custom_title_and_delete_keeps_other_session() {
    let temporary = tempfile::tempdir().unwrap();
    let roots = roots(temporary.path());
    let project = project(temporary.path());
    let transcript = claude_session(&roots, &project, "claude-target", "stale index summary");
    let other = claude_session(&roots, &project, "claude-other", "other summary");
    let sidecar = transcript.with_extension("");
    fs::create_dir_all(sidecar.join("subagents")).unwrap();
    fs::write(sidecar.join("subagents/agent-child.jsonl"), "child history").unwrap();
    let other_sidecar = other.with_extension("");
    fs::create_dir_all(&other_sidecar).unwrap();
    fs::write(other_sidecar.join("tool-output.txt"), "keep me").unwrap();
    let full_title = "x".repeat(200);

    mutate_agent_session(
        SessionProvider::Claude,
        "claude-target",
        &project,
        &roots,
        false,
        &SessionOperation::Rename {
            title: full_title.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        session_title(SessionProvider::Claude, &project, &roots, "claude-target"),
        full_title
    );
    mutate_agent_session(
        SessionProvider::Claude,
        "claude-target",
        &project,
        &roots,
        false,
        &SessionOperation::Rename {
            title: "latest Claude title".into(),
        },
    )
    .unwrap();

    assert_eq!(
        session_title(SessionProvider::Claude, &project, &roots, "claude-target"),
        "latest Claude title"
    );

    mutate_agent_session(
        SessionProvider::Claude,
        "claude-target",
        &project,
        &roots,
        false,
        &SessionOperation::Delete,
    )
    .unwrap();
    assert!(!transcript.exists());
    assert!(!sidecar.exists());
    assert_eq!(
        fs::read_to_string(other_sidecar.join("tool-output.txt")).unwrap(),
        "keep me"
    );
    assert!(other.exists());
    assert_eq!(
        session_title(SessionProvider::Claude, &project, &roots, "claude-other"),
        "other summary"
    );
    let index = other.parent().unwrap().join("sessions-index.json");
    let mut stale: serde_json::Value = serde_json::from_slice(&fs::read(&index).unwrap()).unwrap();
    stale["entries"].as_array_mut().unwrap().push(json!({
        "sessionId": "claude-stale",
        "fullPath": "/outside/the-store.jsonl",
        "fileMtime": 1,
        "summary": "stale index only",
        "projectPath": project,
        "isSidechain": false,
    }));
    fs::write(&index, serde_json::to_vec(&stale).unwrap()).unwrap();
    mutate_agent_session(
        SessionProvider::Claude,
        "claude-stale",
        &project,
        &roots,
        false,
        &SessionOperation::Delete,
    )
    .unwrap();
    assert!(
        !serde_json::from_slice::<serde_json::Value>(&fs::read(index).unwrap()).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["sessionId"] == "claude-stale")
    );
}

#[test]
fn pi_rename_uses_latest_session_info_and_delete_preserves_non_target() {
    let temporary = tempfile::tempdir().unwrap();
    let roots = roots(temporary.path());
    let project = project(temporary.path());
    let target = pi_session(&roots.pi, &project, "pi-target", "initial Pi name");
    let other = pi_session(&roots.pi, &project, "pi-other", "other Pi name");
    let full_title = "p".repeat(200);

    mutate_agent_session(
        SessionProvider::Pi,
        "pi-target",
        &project,
        &roots,
        false,
        &SessionOperation::Rename {
            title: full_title.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        session_title(SessionProvider::Pi, &project, &roots, "pi-target"),
        full_title
    );
    mutate_agent_session(
        SessionProvider::Pi,
        "pi-target",
        &project,
        &roots,
        false,
        &SessionOperation::Rename {
            title: "latest Pi name".into(),
        },
    )
    .unwrap();
    assert_eq!(
        session_title(SessionProvider::Pi, &project, &roots, "pi-target"),
        "latest Pi name"
    );

    mutate_agent_session(
        SessionProvider::Pi,
        "pi-target",
        &project,
        &roots,
        false,
        &SessionOperation::Delete,
    )
    .unwrap();
    assert!(!target.exists());
    assert!(other.exists());
}

#[test]
fn omp_title_slot_wins_then_rename_and_delete_remove_only_its_sidecar() {
    let temporary = tempfile::tempdir().unwrap();
    let roots = roots(temporary.path());
    let project = project(temporary.path());
    let target = omp_session(&roots.omp, &project, "omp-target");
    let target_sidecar = target.with_extension("");
    fs::create_dir_all(&target_sidecar).unwrap();
    fs::write(target_sidecar.join("state.json"), "{}").unwrap();
    let other = omp_session(&roots.omp, &project, "omp-other");
    let other_sidecar = other.with_extension("");
    fs::create_dir_all(&other_sidecar).unwrap();

    assert_eq!(
        session_title(SessionProvider::OhMyPi, &project, &roots, "omp-target"),
        "slot title"
    );
    mutate_agent_session(
        SessionProvider::OhMyPi,
        "omp-target",
        &project,
        &roots,
        false,
        &SessionOperation::Rename {
            title: "renamed OMP title".into(),
        },
    )
    .unwrap();
    assert_eq!(
        session_title(SessionProvider::OhMyPi, &project, &roots, "omp-target"),
        "renamed OMP title"
    );
    let renamed = fs::read(&target).unwrap();
    assert_eq!(renamed.iter().position(|byte| *byte == b'\n'), Some(255));
    assert!(
        mutate_agent_session(
            SessionProvider::OhMyPi,
            "omp-target",
            &project,
            &roots,
            false,
            &SessionOperation::Rename {
                title: "界".repeat(180)
            },
        )
        .is_err()
    );
    assert_eq!(
        fs::read(&target).unwrap(),
        renamed,
        "oversized names must not silently truncate or partially rewrite OMP history"
    );

    mutate_agent_session(
        SessionProvider::OhMyPi,
        "omp-target",
        &project,
        &roots,
        false,
        &SessionOperation::Delete,
    )
    .unwrap();
    assert!(!target.exists());
    assert!(!target_sidecar.exists());
    assert!(other.exists());
    assert!(other_sidecar.exists());
}

#[test]
fn grok_manual_title_round_trips_and_offline_delete_keeps_other_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let roots = roots(temporary.path());
    let project = project(temporary.path());
    let target = grok_session(&roots.grok, &project, "grok-target", "generated target");
    let other = grok_session(&roots.grok, &project, "grok-other", "generated other");
    let full_title = "g".repeat(100);

    mutate_agent_session(
        SessionProvider::Grok,
        "grok-target",
        &project,
        &roots,
        false,
        &SessionOperation::Rename {
            title: full_title.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        session_title(SessionProvider::Grok, &project, &roots, "grok-target"),
        full_title
    );
    assert!(
        mutate_agent_session(
            SessionProvider::Grok,
            "grok-target",
            &project,
            &roots,
            false,
            &SessionOperation::Rename {
                title: "g".repeat(101)
            },
        )
        .is_err()
    );
    assert_eq!(
        session_title(SessionProvider::Grok, &project, &roots, "grok-target"),
        full_title
    );
    mutate_agent_session(
        SessionProvider::Grok,
        "grok-target",
        &project,
        &roots,
        false,
        &SessionOperation::Delete,
    )
    .unwrap();
    assert!(!target.exists());
    assert!(other.exists());
}

#[cfg(unix)]
#[test]
fn mutation_rejects_symlinked_history_without_touching_its_target() {
    let temporary = tempfile::tempdir().unwrap();
    let roots = roots(temporary.path());
    let project = project(temporary.path());
    let external = temporary.path().join("external.jsonl");
    fs::write(
        &external,
        format!(
            "{}\n",
            json!({"type":"session","version":3,"id":"pi-target","cwd":project})
        ),
    )
    .unwrap();
    fs::create_dir_all(roots.pi.join("encoded")).unwrap();
    std::os::unix::fs::symlink(&external, roots.pi.join("encoded/pi-target.jsonl")).unwrap();

    assert!(
        mutate_agent_session(
            SessionProvider::Pi,
            "pi-target",
            &project,
            &roots,
            false,
            &SessionOperation::Delete,
        )
        .is_err()
    );
    assert!(external.exists());
}

#[test]
fn mutation_refuses_an_open_target_and_rejects_invalid_titles_before_writing() {
    let temporary = tempfile::tempdir().unwrap();
    let roots = roots(temporary.path());
    let project = project(temporary.path());
    let target = pi_session(&roots.pi, &project, "pi-target", "initial Pi name");
    let open = fs::File::open(&target).unwrap();

    assert!(
        mutate_agent_session(
            SessionProvider::Pi,
            "pi-target",
            &project,
            &roots,
            false,
            &SessionOperation::Rename {
                title: "blocked".into()
            },
        )
        .is_err()
    );
    drop(open);
    assert!(
        mutate_agent_session(
            SessionProvider::Pi,
            "pi-target",
            &project,
            &roots,
            false,
            &SessionOperation::Rename {
                title: "bad\nname".into()
            },
        )
        .is_err()
    );
    assert_eq!(
        session_title(SessionProvider::Pi, &project, &roots, "pi-target"),
        "initial Pi name"
    );
}
