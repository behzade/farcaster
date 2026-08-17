use super::*;
use std::{error::Error, fs, io::Write as _};
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn session(root: &Path, file: &str, cwd: &Path, name: Option<&str>, message: &str) -> TestResult {
    session_with_parent(root, file, cwd, name, message, None)
}

fn session_with_parent(
    root: &Path,
    file: &str,
    cwd: &Path,
    name: Option<&str>,
    message: &str,
    parent: Option<&str>,
) -> TestResult {
    let directory = root.join("custom/nested");
    fs::create_dir_all(&directory)?;
    let mut lines = vec![
        serde_json::json!({"type":"session","version":3,"id":file,"timestamp":"2026-01-02T00:00:00Z","cwd":cwd,"parentSession":parent}),
    ];
    lines.push(serde_json::json!({"type":"unknown","data":true}));
    lines.push(serde_json::json!({"type":"message","message":{"role":"user","content":message}}));
    if let Some(name) = name {
        lines.push(serde_json::json!({"type":"session_info","name":name}));
    }
    fs::write(
        directory.join(format!("{file}.jsonl")),
        lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    Ok(())
}

fn nested_session(
    root: &Path,
    root_file: &str,
    id: &str,
    cwd: &Path,
    name: &str,
    message: &str,
) -> TestResult {
    let directory = root
        .join("custom/nested")
        .join(root_file)
        .join("agent/run-0");
    fs::create_dir_all(&directory)?;
    let lines = [
        serde_json::json!({"type":"session","version":3,"id":id,"timestamp":"2026-01-02T00:00:00Z","cwd":cwd}),
        serde_json::json!({"type":"message","message":{"role":"user","content":message}}),
        serde_json::json!({"type":"session_info","name":name}),
    ];
    fs::write(
        directory.join("session.jsonl"),
        lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    Ok(())
}

#[test]
fn override_resolution_order_is_explicit() {
    let cwd = Path::new("/work");
    assert_eq!(
        session_root_from_at(
            cwd,
            Some(Path::new("/h")),
            Some(Path::new("/a")),
            Some(Path::new("/s"))
        ),
        Ok(PathBuf::from("/s"))
    );
    assert_eq!(
        session_root_from_at(cwd, Some(Path::new("/h")), Some(Path::new("/a")), None),
        Ok(PathBuf::from("/a/sessions"))
    );
    assert_eq!(
        session_root_from_at(cwd, Some(Path::new("/h")), None, None),
        Ok(PathBuf::from("/h/.pi/agent/sessions"))
    );
    assert_eq!(
        session_root_from_at(cwd, None, None, Some(Path::new("relative/sessions"))),
        Ok(PathBuf::from("/work/relative/sessions"))
    );
}

#[test]
fn only_a_missing_session_root_is_exhaustive_empty() -> TestResult {
    let temp = tempdir()?;
    let missing = temp.path().join("missing");
    assert!(discover_in_with_status(&missing, "")?.exhaustive);

    let blocker = temp.path().join("not-a-directory");
    fs::write(&blocker, "file")?;
    let invalid_child = blocker.join("sessions");
    assert!(discover_in_with_status(&invalid_child, "").is_err());
    Ok(())
}

#[test]
fn malformed_candidate_marks_discovery_partial_instead_of_prunable() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(root.path(), "valid", project.path(), Some("Valid"), "hello")?;
    fs::write(root.path().join("broken.jsonl"), b"not json\n")?;

    let mut cache = DiscoveryCache::default();
    let discovery = discover_in_cached(root.path(), "", &mut cache)?;
    assert!(!discovery.exhaustive);
    assert_eq!(cache.candidates.len(), 1);
    let second = discover_in_cached(root.path(), "", &mut cache)?;
    assert!(!second.exhaustive);
    assert_eq!(cache.candidates.len(), 1);
    assert!(
        discovery
            .sessions
            .iter()
            .any(|session| session.id == "valid")
    );
    Ok(())
}

#[test]
fn discovers_all_projects_and_name_or_message_fallback() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    let other = tempdir()?;
    session(
        root.path(),
        "named",
        project.path(),
        Some("Named run"),
        "first text",
    )?;
    session(
        root.path(),
        "fallback",
        project.path(),
        None,
        "A useful fallback title continues",
    )?;
    session(
        root.path(),
        "other",
        other.path(),
        Some("Other run"),
        "visible",
    )?;
    let sessions = discover_in(root.path(), "")?;
    assert_eq!(sessions.len(), 3);
    assert!(sessions.iter().all(|item| item.path.is_absolute()));
    assert!(sessions.iter().all(|item| item.project.is_absolute()));
    assert!(sessions.iter().any(|item| item.title == "Named run"));
    assert!(sessions.iter().any(|item| {
        item.title == "Other run"
            && item.project == other.path().canonicalize().expect("project path")
    }));
    assert!(
        sessions
            .iter()
            .any(|item| item.title.starts_with("A useful fallback"))
    );
    Ok(())
}

#[test]
fn search_is_case_insensitive_and_malformed_entries_do_not_poison() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(root.path(), "one", project.path(), None, "Alpha Beta")?;
    let path = root.path().join("custom/nested/one.jsonl");
    let mut content = fs::read_to_string(&path)?;
    content.push_str("\n{broken\n");
    fs::write(path, content)?;
    assert_eq!(discover_in(root.path(), "bEtA")?.len(), 1);
    Ok(())
}

#[test]
fn stale_incomplete_children_are_not_reported_as_running_forever() {
    let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10_000);
    assert!(recently_running(
        true,
        now - std::time::Duration::from_secs(30),
        now
    ));
    assert!(!recently_running(
        true,
        now - RUNNING_ACTIVITY_TIMEOUT - std::time::Duration::from_secs(1),
        now
    ));
    assert!(!recently_running(false, now, now));
}

#[test]
fn discovery_tracks_running_children_from_the_last_message_lifecycle() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(root.path(), "running", project.path(), None, "Question")?;
    session(root.path(), "done", project.path(), None, "Question")?;
    let directory = root.path().join("custom/nested");
    fs::OpenOptions::new()
        .append(true)
        .open(directory.join("running.jsonl"))?
        .write_all(
            format!(
                "\n{}\n{}",
                serde_json::json!({
                    "type": "message",
                    "message": {"role": "assistant", "stopReason": "toolUse"}
                }),
                serde_json::json!({
                    "type": "message",
                    "message": {"role": "toolResult"}
                })
            )
            .as_bytes(),
        )?;
    fs::OpenOptions::new()
        .append(true)
        .open(directory.join("done.jsonl"))?
        .write_all(
            format!(
                "\n{}",
                serde_json::json!({
                    "type": "message",
                    "message": {"role": "assistant", "stopReason": "stop"}
                })
            )
            .as_bytes(),
        )?;

    let sessions = discover_in(root.path(), "")?;
    assert!(
        sessions
            .iter()
            .find(|session| session.id == "running")
            .is_some_and(|session| session.is_running)
    );
    assert!(
        !sessions
            .iter()
            .find(|session| session.id == "done")
            .is_some_and(|session| session.is_running)
    );
    Ok(())
}

#[test]
fn discovery_sums_token_and_cost_usage_from_assistant_messages() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(root.path(), "usage", project.path(), None, "Question")?;
    let path = root.path().join("custom/nested/usage.jsonl");
    let mut content = fs::read_to_string(&path)?;
    content.push_str(&format!(
        "\n{}",
        serde_json::json!({
            "type": "message",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Answer"}],
                "usage": {
                    "input": 1000,
                    "output": 200,
                    "cacheRead": 3000,
                    "cacheWrite": 50,
                    "totalTokens": 4250,
                    "cost": {"total": 0.123456}
                }
            }
        })
    ));
    fs::write(path, content)?;

    let sessions = discover_in(root.path(), "")?;
    assert_eq!(
        sessions[0].usage,
        UsageSummary {
            input: 1000,
            output: 200,
            cache_read: 3000,
            cache_write: 50,
            total: 4250,
            cost_micros: 123_456,
        }
    );
    Ok(())
}

#[test]
fn official_path_parent_resolves_to_id_and_keeps_live_hierarchy() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(
        root.path(),
        "root-id",
        project.path(),
        Some("Main"),
        "ordinary",
    )?;
    let parent_path = root.path().join("custom/nested/root-id.jsonl");
    session_with_parent(
        root.path(),
        "child-id",
        project.path(),
        Some("subagent-worker"),
        "Needle",
        parent_path.to_str(),
    )?;
    let child_path = root.path().join("custom/nested/child-id.jsonl");
    fs::OpenOptions::new()
        .append(true)
        .open(&child_path)?
        .write_all(
            format!(
                "\n{}",
                serde_json::json!({
                    "type": "message",
                    "message": {"role": "toolResult"}
                })
            )
            .as_bytes(),
        )?;

    let filtered = discover_in(root.path(), "needle")?;
    assert_eq!(filtered.len(), 2);
    let child = filtered
        .iter()
        .find(|session| session.id == "child-id")
        .expect("child should be retained");
    assert_eq!(child.parent_session.as_deref(), Some("root-id"));
    assert!(child.is_running);
    assert_eq!(
        root_session_for_path(&filtered, Some(child.path.as_path()))
            .map(|session| session.id.as_str()),
        Some("root-id")
    );
    assert_eq!(
        descendant_sessions(&filtered, "root-id")
            .iter()
            .map(|(session, depth)| (session.id.as_str(), *depth))
            .collect::<Vec<_>>(),
        vec![("child-id", 1)]
    );

    let cached = SessionSummary::from_cached(
        "cached-child".into(),
        child_path,
        project.path().to_owned(),
        "Cached child".into(),
        String::new(),
        String::new(),
        parent_path.to_str().map(str::to_owned),
        SystemTime::now(),
        0,
        UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    assert_eq!(cached.parent_session.as_deref(), Some("root-id"));
    Ok(())
}

#[test]
fn missing_path_parent_stays_an_orphan_id_and_header_reads_are_bounded() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    let missing = root.path().join("2026-01-01_missing-parent-id.jsonl");
    session_with_parent(
        root.path(),
        "child",
        project.path(),
        None,
        "work",
        missing.to_str(),
    )?;
    let sessions = discover_in(root.path(), "")?;
    let child = sessions
        .iter()
        .find(|session| session.id == "child")
        .expect("orphan child should remain discoverable");
    assert_eq!(child.parent_session.as_deref(), Some("missing-parent-id"));
    assert!(root_sessions(&sessions).is_empty());
    assert!(
        resolve_parent_session(&child.path, "/")
            .is_some_and(|id| id.starts_with("unresolved-parent-"))
    );

    let oversized = root.path().join("oversized.jsonl");
    fs::write(&oversized, vec![b'x'; MAX_HEADER_BYTES + 1])?;
    assert_eq!(session_header_id(&oversized), None);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_parent_path_resolves_the_referenced_header_id() -> TestResult {
    use std::os::unix::fs::symlink;

    let root = tempdir()?;
    let project = tempdir()?;
    session(root.path(), "root-id", project.path(), None, "main")?;
    let parent = root.path().join("custom/nested/root-id.jsonl");
    let alias = root.path().join("parent-link.jsonl");
    symlink(parent, &alias)?;
    session_with_parent(
        root.path(),
        "child-id",
        project.path(),
        None,
        "child",
        alias.to_str(),
    )?;

    let sessions = discover_in(root.path(), "")?;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == "child-id")
            .and_then(|session| session.parent_session.as_deref()),
        Some("root-id")
    );
    Ok(())
}

#[test]
fn discovery_carries_transient_agent_activity_without_persisting_it() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(
        root.path(),
        "child",
        project.path(),
        Some("subagent-worker-generated"),
        "Implement the activity panel",
    )?;
    let path = root.path().join("custom/nested/child.jsonl");
    let entries = [
        serde_json::json!({
            "type":"message",
            "message":{"role":"assistant","stopReason":"toolUse","content":[
                {"type":"toolCall","id":"edit-1","name":"edit","arguments":{
                    "path":"src/main.rs","oldText":"before","newText":"after"
                }}
            ]}
        }),
        serde_json::json!({
            "type":"message","message":{
                "role":"toolResult","toolCallId":"edit-1","toolName":"edit","isError":false,
                "details":{"patch":"@@\n-before\n+after\n"}
            }
        }),
        serde_json::json!({
            "type":"message",
            "message":{"role":"assistant","stopReason":"stop","content":[]}
        }),
    ];
    let mut file = fs::OpenOptions::new().append(true).open(path)?;
    writeln!(file)?;
    for entry in entries {
        writeln!(file, "{entry}")?;
    }

    let discovery = discover_in_with_status(root.path(), "")?;
    let activity = discovery.activities.get("child").expect("activity");
    assert_eq!(activity.role, "worker");
    assert_eq!(activity.activity, "Implement the activity panel");
    assert_eq!(activity.tool_call_count, 1);
    assert!(activity.current_tool.is_none());
    assert_eq!(
        activity.recent_tool.as_ref().map(|tool| tool.name.as_str()),
        Some("edit")
    );
    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Completed(
            crate::agent_activity::AgentOutcome::Complete
        )
    );
    assert_eq!(
        activity.changed_paths[0].path,
        project.path().canonicalize()?.join("src/main.rs")
    );
    assert_eq!(activity.file_mutations.len(), 1);
    assert!(matches!(
        &activity.file_mutations[0].kind,
        crate::agent_activity::FileMutationKind::Edit { patch, complete: true }
            if patch.contains("+after")
    ));
    Ok(())
}

#[test]
fn discovery_aggregates_only_the_active_branch_from_an_external_session() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    let directory = root.path().join("custom/nested");
    fs::create_dir_all(&directory)?;
    let entries = [
        serde_json::json!({"type":"session","version":3,"id":"external","timestamp":"2026-01-02T00:00:00Z","cwd":project.path()}),
        serde_json::json!({"type":"message","id":"root","parentId":null,"message":{"role":"user","content":"change one branch"}}),
        serde_json::json!({"type":"message","id":"old-call","parentId":"root","timestamp":"2026-01-02T00:00:01Z","message":{"role":"assistant","stopReason":"toolUse","content":[
            {"type":"toolCall","id":"old-edit","name":"edit","arguments":{"path":"old.txt","oldText":"a","newText":"b"}}
        ]}}),
        serde_json::json!({"type":"message","id":"old-result","parentId":"old-call","message":{"role":"toolResult","toolCallId":"old-edit","toolName":"edit","isError":false,"details":{"patch":"@@\n-a\n+b\n"}}}),
        serde_json::json!({"type":"message","id":"current-call","parentId":"root","timestamp":"2026-01-02T00:00:02Z","message":{"role":"assistant","stopReason":"toolUse","content":[
            {"type":"toolCall","id":"current-write","name":"write","arguments":{"path":"current.txt","content":"current\n"}}
        ]}}),
        serde_json::json!({"type":"message","id":"current-result","parentId":"current-call","message":{"role":"toolResult","toolCallId":"current-write","toolName":"write","isError":false}}),
    ];
    fs::write(
        directory.join("external.jsonl"),
        entries
            .into_iter()
            .map(|entry| entry.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )?;

    let discovery = discover_in_with_status(root.path(), "")?;
    let activity = discovery.activities.get("external").expect("activity");
    assert_eq!(activity.file_mutations.len(), 1);
    assert_eq!(
        activity.file_mutations[0].path,
        project.path().canonicalize()?.join("current.txt")
    );
    assert!(matches!(
        activity.file_mutations[0].kind,
        crate::agent_activity::FileMutationKind::Write { .. }
    ));
    Ok(())
}

#[test]
fn child_search_keeps_its_root_and_hierarchy_is_stable() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    session(
        root.path(),
        "root",
        project.path(),
        Some("Main"),
        "ordinary",
    )?;
    nested_session(
        root.path(),
        "root",
        "child",
        project.path(),
        "subagent-reviewer-long-id",
        "Needle",
    )?;
    session_with_parent(
        root.path(),
        "grandchild",
        project.path(),
        Some("subagent-worker-long-id"),
        "Nested",
        Some("child"),
    )?;
    session_with_parent(
        root.path(),
        "orphan",
        project.path(),
        Some("subagent-worker-orphan-1"),
        "Detached",
        Some("missing"),
    )?;

    let sessions = discover_in(root.path(), "needle")?;
    assert_eq!(sessions.len(), 3);
    let roots = root_sessions(&sessions);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, "root");
    let child = sessions
        .iter()
        .find(|session| session.id == "child")
        .expect("matching child should remain");
    assert_eq!(child.parent_session.as_deref(), Some("root"));
    assert_eq!(
        root_session_for_path(&sessions, Some(child.path.as_path()))
            .map(|session| session.id.as_str()),
        Some("root")
    );

    let all = discover_in(root.path(), "")?;
    assert_eq!(
        root_sessions(&all)
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["root"]
    );
    let descendants = descendant_sessions(&all, "root");
    assert_eq!(
        descendants
            .iter()
            .map(|(session, depth)| (session.id.as_str(), *depth))
            .collect::<Vec<_>>(),
        vec![("child", 1), ("grandchild", 2)]
    );
    Ok(())
}

#[test]
fn truncated_long_sessions_do_not_infer_lifecycle_from_the_prefix() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    let path = root.path().join("long.jsonl");
    let mut file = fs::File::create(&path)?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"type":"session","version":3,"id":"long","timestamp":"2026-01-02T00:00:00Z","cwd":project.path()})
    )?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"pending","name":"edit","arguments":{"path":"src/lib.rs"}}]}})
    )?;
    for _ in 1..MAX_LINES_PER_FILE {
        writeln!(file, "{}", serde_json::json!({"type":"unknown"}))?;
    }
    writeln!(
        file,
        "{}",
        serde_json::json!({"type":"message","message":{"role":"assistant","stopReason":"stop","content":[]}})
    )?;

    let discovery = discover_in_with_status(root.path(), "")?;
    let session = discovery
        .sessions
        .iter()
        .find(|session| session.id == "long")
        .expect("session");
    let activity = discovery.activities.get("long").expect("activity");
    assert!(!session.is_running);
    assert!(activity.limited);
    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Unknown
    );
    assert!(activity.current_tool.is_none());
    Ok(())
}

#[test]
fn bounded_discovery_keeps_the_newest_candidates_and_reports_partial_results() -> TestResult {
    let root = tempdir()?;
    let project = tempdir()?;
    for index in 0..MAX_CANDIDATES {
        session(
            root.path(),
            &format!("old-{index:04}"),
            project.path(),
            None,
            "old",
        )?;
    }
    std::thread::sleep(std::time::Duration::from_millis(5));
    session(root.path(), "newest-child", project.path(), None, "new")?;

    let discovery = discover_in_with_status(root.path(), "")?;
    assert!(!discovery.exhaustive);
    assert_eq!(discovery.sessions.len(), MAX_CANDIDATES);
    assert!(
        discovery
            .sessions
            .iter()
            .any(|session| session.id == "newest-child")
    );
    Ok(())
}

#[test]
fn history_follows_the_current_branch_and_projects_display_entries() {
    let entries = vec![
        serde_json::json!({"type":"message","id":"one","parentId":null,"message":{"role":"user","content":"root"}}),
        serde_json::json!({"type":"message","id":"old","parentId":"one","message":{"role":"assistant","content":[{"type":"text","text":"old branch"}]}}),
        serde_json::json!({"type":"message","id":"two","parentId":"one","message":{"role":"assistant","content":[{"type":"text","text":"current"}]}}),
        serde_json::json!({"type":"custom_message","id":"three","parentId":"two","customType":"note","content":"visible","display":true}),
    ];

    let history = project_history(&entries);

    assert_eq!(history.len(), 3);
    assert_eq!(history[0]["content"], "root");
    assert_eq!(history[1]["content"][0]["text"], "current");
    assert_eq!(history[2]["role"], "custom");
}

#[test]
fn history_matches_pi_compaction_order() {
    let entries = vec![
        serde_json::json!({"type":"message","id":"one","parentId":null,"message":{"role":"user","content":"summarized"}}),
        serde_json::json!({"type":"message","id":"two","parentId":"one","message":{"role":"user","content":"kept"}}),
        serde_json::json!({"type":"compaction","id":"three","parentId":"two","summary":"summary","firstKeptEntryId":"two"}),
        serde_json::json!({"type":"message","id":"four","parentId":"three","message":{"role":"user","content":"after"}}),
    ];

    let history = project_history(&entries);

    assert_eq!(history.len(), 3);
    assert_eq!(history[0]["role"], "compactionSummary");
    assert_eq!(history[1]["content"], "kept");
    assert_eq!(history[2]["content"], "after");
}

#[test]
fn loaded_history_includes_active_branch_model_and_effort() -> TestResult {
    let directory = tempdir()?;
    let path = directory.path().join("session.jsonl");
    fs::write(
        &path,
        [
            serde_json::json!({"type":"session","version":3,"id":"session","cwd":"/project"}),
            serde_json::json!({"type":"model_change","id":"one","parentId":null,"provider":"openai-codex","modelId":"gpt-luna"}),
            serde_json::json!({"type":"thinking_level_change","id":"two","parentId":"one","thinkingLevel":"high"}),
            serde_json::json!({"type":"message","id":"three","parentId":"two","message":{"role":"user","content":"hello"}}),
        ]
        .into_iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n"),
    )?;

    let history = load_history(&path)?;

    assert_eq!(
        history.model,
        Some(("openai-codex".into(), "gpt-luna".into()))
    );
    assert_eq!(history.thinking_level.as_deref(), Some("high"));
    assert_eq!(history.messages.len(), 1);
    Ok(())
}
