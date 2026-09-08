use std::path::Path;

use super::*;

fn summary(path: &Path, modified: SystemTime, is_running: bool) -> SessionSummary {
    SessionSummary::from_cached(
        "external".into(),
        path.to_path_buf(),
        PathBuf::from("/project"),
        "External".into(),
        String::new(),
        String::new(),
        None,
        modified,
        0,
        crate::sessions::UsageSummary::default(),
        false,
        is_running,
        String::new(),
    )
}

#[test]
fn worker_families_join_foreign_locators_by_harness_and_project() {
    let mut parent = summary(
        Path::new("/sessions/parent.jsonl"),
        SystemTime::now(),
        false,
    );
    parent.id = "parent-id".into();
    parent.harness = "pi".into();
    let mut child = summary(Path::new("/locators/child"), SystemTime::now(), false);
    child.id = "child-id".into();
    child.harness = "opencode2".into();
    let mut unrelated = child.clone();
    unrelated.harness = "codex-cli".into();
    let link = crate::agents::WorkerFamilyLink {
        project: PathBuf::from("/project"),
        child_backend: "opencode2".into(),
        child_session: "child-id".into(),
        parent_backend: "pi".into(),
        parent_session: "/sessions/parent.jsonl".into(),
        execution: None,
    };
    let mut sessions = vec![parent, child, unrelated];
    apply_worker_families(&mut sessions, &[link]);
    assert_eq!(sessions[1].parent_session.as_deref(), Some("parent-id"));
    assert!(sessions[2].parent_session.is_none());
}

#[test]
fn legacy_worker_execution_is_recovered_only_for_the_matching_child() {
    let mut child = summary(Path::new("/locators/child"), SystemTime::now(), false);
    child.harness = "opencode2".into();
    let link = agents::WorkerFamilyLink {
        project: child.project.clone(),
        child_backend: child.harness.clone(),
        child_session: child.id.clone(),
        parent_backend: "pi".into(),
        parent_session: "/sessions/parent.jsonl".into(),
        execution: None,
    };
    let mut other = child.clone();
    other.harness = "codex-cli".into();
    let mut sessions = vec![other, child];
    let mut calls = 0;
    recover_worker_execution(&mut sessions, std::slice::from_ref(&link), |harness, _| {
        assert_eq!(harness, "opencode2");
        calls += 1;
        Ok(LoadedHistory {
            messages: vec![],
            model: Some(("opencode-go".into(), "glm-5.3-flash".into())),
            thinking_level: Some("high".into()),
            pending_question: None,
        })
    });
    assert_eq!(calls, 1);
    assert!(sessions[0].model.is_none());
    assert_eq!(
        sessions[1].model,
        Some(("opencode-go".into(), "glm-5.3-flash".into()))
    );
    assert_eq!(sessions[1].thinking_level.as_deref(), Some("high"));
    let saved = agents::WorkerFamilyLink {
        execution: Some(agents::WorkerExecution {
            harness: "opencode2".into(),
            provider: "opencode-go".into(),
            model: "glm-5.3-flash".into(),
            effort: Some("high".into()),
        }),
        ..link
    };
    sessions[1].model = None;
    recover_worker_execution(&mut sessions, &[saved], |_, _| {
        panic!("saved identity must not reload history")
    });
}

#[test]
fn catalog_sync_seeds_the_remaining_activity_deadline() {
    let wall_now = SystemTime::now();
    let now = Instant::now();
    let session = summary(
        Path::new("/sessions/external.jsonl"),
        wall_now - Duration::from_secs(5),
        true,
    );
    let mut activity = ExternalActivityTracker::default();

    activity.sync_catalog(&[session], true, &HashSet::new(), now, wall_now);

    assert!(!activity.take_expired(now + RUNNING_ACTIVITY_TIMEOUT - Duration::from_secs(6)));
    assert!(activity.take_expired(now + RUNNING_ACTIVITY_TIMEOUT - Duration::from_secs(5)));
}

#[test]
fn cached_sessions_get_truthful_limited_activity_fallbacks() {
    let session = summary(
        Path::new("/sessions/external.jsonl"),
        SystemTime::now(),
        false,
    );
    let mut activities = HashMap::new();

    add_limited_activity_fallbacks(&mut activities, &[session]);

    let activity = activities.get("external").expect("fallback activity");
    assert!(activity.limited);
    assert_eq!(
        activity.lifecycle,
        crate::agent_activity::AgentLifecycle::Unknown
    );
    assert_eq!(activity.role, "External");
}

#[test]
fn import_preview_skips_sessions_already_in_the_catalog() {
    let known = HashSet::from([PathBuf::from("/sessions/known.jsonl")]);
    let known_session = summary(Path::new("/sessions/known.jsonl"), SystemTime::now(), false);
    let mut unknown = summary(Path::new("/sessions/new.jsonl"), SystemTime::now(), false);
    unknown.id = "new".into();

    let candidates = unknown_import_candidates(vec![known_session, unknown.clone()], &known);

    assert_eq!(candidates, vec![unknown]);
}

#[test]
fn parsed_activity_wins_over_a_limited_fallback() {
    let session = summary(
        Path::new("/sessions/external.jsonl"),
        SystemTime::now(),
        true,
    );
    let parsed = ActivityBuilder::default().finish(
        session.id.clone(),
        session.path.clone(),
        &session.title,
        "Working now",
        session.usage,
        session.modified,
        session.modified,
        true,
        false,
    );
    let mut activities = HashMap::from([(session.id.clone(), parsed.clone())]);

    add_limited_activity_fallbacks(&mut activities, &[session]);

    assert_eq!(activities.get("external"), Some(&parsed));
}
