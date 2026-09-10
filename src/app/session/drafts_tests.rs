use super::*;

#[test]
fn empty_startup_draft_stays_deleted_after_late_composer_save()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::app::infrastructure::persistence::{ComposerRecord, StateStore};

    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let project = temp.path().canonicalize()?;
    let mut store = StateStore::open_at(&database)?;
    let draft = DraftSession::with_id("pi".into(), "startup".into(), project.clone());
    let id = store.allocate_app_session_id(&draft)?;
    let mut registry = store.load_registry()?;
    assert!(sync_materialized_draft(
        &mut registry.drafts,
        "startup",
        id,
        &project,
        "pi",
        false,
    ));
    store.save_registry(&registry)?;
    // A queued composer write must not recreate the draft after quit removes it.
    store.save_composer_session(&ComposerRecord {
        target: draft_target("startup"),
        ..Default::default()
    })?;
    drop(store);

    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.load_registry()?.drafts.is_empty());
    assert!(reopened.load_composer_sessions()?.is_empty());
    Ok(())
}

#[test]
fn project_choices_include_registered_and_current_worktrees() {
    let temp = tempfile::tempdir().expect("temporary project root");
    let project = temp.path().join("project");
    let other = temp.path().join("other");
    let worktree = temp.path().join("worktree");
    let worktree_git_dir = project.join(".git/worktrees/feature");
    std::fs::create_dir_all(&worktree_git_dir).expect("worktree metadata");
    std::fs::create_dir_all(&worktree).expect("worktree directory");
    std::fs::write(worktree_git_dir.join("commondir"), "../..\n")
        .expect("worktree common directory pointer");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git_dir.display()),
    )
    .expect("worktree git pointer");
    let registered = vec![project.clone(), worktree.clone(), other.clone()];

    assert_eq!(
        available_projects(&registered, &other),
        vec![other.clone(), worktree.clone(), project.clone()]
    );
    assert_eq!(
        available_projects(&registered, &worktree),
        vec![worktree, project, other]
    );
}

#[test]
fn drafts_materialize_only_when_leaving_a_composer_with_content() {
    let project = PathBuf::from("/project");
    let mut drafts = Vec::new();

    assert!(!draft_has_content(
        &crate::app::composer::sessions::ComposerSnapshot::default()
    ));
    assert!(!draft_has_content(
        &crate::app::composer::sessions::ComposerSnapshot::new("   ".into(), 3, 3..3)
    ));
    assert!(draft_has_content(
        &crate::app::composer::sessions::ComposerSnapshot::new("work".into(), 4, 4..4)
    ));
    assert!(!sync_materialized_draft(
        &mut drafts,
        "ephemeral",
        42,
        &project,
        "codex-cli",
        false,
    ));
    assert!(drafts.is_empty());
    assert!(sync_materialized_draft(
        &mut drafts,
        "ephemeral",
        42,
        &project,
        "codex-cli",
        true,
    ));
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].id, "ephemeral");
    assert_eq!(drafts[0].app_session_id, 42);
    assert_eq!(drafts[0].harness, "codex-cli");
    assert!(!sync_materialized_draft(
        &mut drafts,
        "ephemeral",
        42,
        &project,
        "codex-cli",
        true,
    ));
    assert!(sync_materialized_draft(
        &mut drafts,
        "ephemeral",
        42,
        &project,
        "codex-cli",
        false,
    ));
    assert!(drafts.is_empty());
}

#[test]
fn provisional_title_uses_first_nonblank_bounded_prompt_line() {
    assert_eq!(
        provisional_session_title("\n  Fix the composer submission flow.\nMore detail"),
        Some("Fix the composer submission flow".into())
    );
    assert_eq!(provisional_session_title("   \n"), None);
    assert_eq!(
        provisional_session_title(
            "one two three four five six seven eight nine ten eleven twelve thirteen"
        ),
        Some("one two three four five six seven eight nine ten eleven twelve".into())
    );
}

#[test]
fn submitted_pathless_drafts_keep_their_pending_identity() {
    let draft = DraftSession {
        id: "pending".into(),
        app_session_id: 1,
        harness: "pi".into(),
        project: PathBuf::from("/project"),
        created_ms: 1,
        submitted: true,
        session_path: None,
        title: Some("Pending session".into()),
    };

    assert_eq!(
        submitted_draft_associations(&[draft]),
        HashMap::from([("pending".into(), None)])
    );
}

#[test]
fn submitted_a_and_selected_empty_b_keep_distinct_identity() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let mut submitted = HashMap::new();

    assert_eq!(
        establish_submission(&mut submitted, "draft:a", true, Some(path.clone()),),
        Some("a".into())
    );
    let selected_draft = "b";

    assert_eq!(selected_draft, "b");
    assert_eq!(submitted.get("a"), Some(&Some(path)));
    assert_eq!(
        resolved_draft_status("a", &submitted, &HashMap::new()),
        "Working"
    );
    assert_eq!(
        resolved_draft_status("b", &submitted, &HashMap::new()),
        "Draft"
    );
}

#[test]
fn later_draft_status_fills_only_an_established_submission() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let mut submitted = HashMap::new();
    establish_submission(&mut submitted, "draft:a", true, None);

    assert_eq!(
        fill_session_association(&mut submitted, "draft:a", Some(&path)),
        Some(path.clone())
    );
    assert_eq!(
        fill_session_association(&mut submitted, "draft:b", Some(&path)),
        None
    );
    assert!(!submitted.contains_key("b"));
}

#[test]
fn submitted_draft_status_prefers_draft_then_associated_session_then_fallback() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let submitted = HashMap::from([("a".into(), Some(path.clone()))]);
    let session_key = session_target(&path);
    let mut statuses = HashMap::from([(session_key, "Needs input".into())]);

    assert_eq!(
        resolved_draft_status("a", &submitted, &statuses),
        "Needs input"
    );
    statuses.insert(draft_target("a"), "Failed".into());
    assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Failed");
    statuses.remove(&draft_target("a"));
    statuses.insert(session_target(&path), "Done".into());
    assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Done");
    statuses.insert(session_target(&path), "Working".into());
    assert_eq!(resolved_draft_status("a", &submitted, &statuses), "Working");
}

#[test]
fn accepted_draft_with_exact_path_reconciles_after_store_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, time::SystemTime};

    use tempfile::tempdir;

    use crate::{
        app::infrastructure::persistence::StateStore,
        projects::Registry,
        sessions::{SessionSummary, UsageSummary},
    };

    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let session = temp.path().join("a.jsonl");
    fs::write(&session, "{}")?;
    let project = project.canonicalize()?;
    let session = session.canonicalize()?;
    let mut drafts = vec![DraftSession {
        id: "a".into(),
        app_session_id: 1,
        harness: "pi".into(),
        project: project.clone(),
        created_ms: 1,
        submitted: false,
        session_path: None,
        title: None,
    }];
    let mut submitted = HashMap::new();

    establish_submission(&mut submitted, "draft:a", true, Some(session.clone()));
    assert!(update_persisted_submission(
        &mut drafts,
        "a",
        Some(&session)
    ));
    let database = temp.path().join("gui-state.sqlite3");
    {
        let mut store = StateStore::open_at(&database)?;
        store.save_registry(&Registry {
            projects: vec![project.clone()],
            excluded_projects: Vec::new(),
            drafts,
        })?;
        store.replace_sessions(&[SessionSummary::from_cached(
            "session-a".into(),
            session.clone(),
            project,
            "Session A".into(),
            "hello".into(),
            "2026-08-15T00:00:00Z".into(),
            None,
            SystemTime::now(),
            1,
            UsageSummary::default(),
            false,
            false,
            "session a hello".into(),
        )])?;
    }

    let store = StateStore::open_at(&database)?;
    let restarted = store.load_registry()?;
    let restarted_submitted = submitted_draft_associations(&restarted.drafts);
    let catalog = store.cached_sessions("")?;

    assert_eq!(
        reconciliation_candidates(
            &restarted_submitted,
            catalog.iter().map(|summary| summary.path.as_path()),
        ),
        vec![("a".into(), session)]
    );
    Ok(())
}

#[test]
fn accepted_draft_without_a_path_is_never_durable() {
    let mut drafts = vec![DraftSession {
        id: "a".into(),
        app_session_id: 1,
        harness: "pi".into(),
        project: PathBuf::from("/project"),
        created_ms: 1,
        submitted: false,
        session_path: None,
        title: None,
    }];
    let mut submitted = HashMap::new();

    establish_submission(&mut submitted, "draft:a", true, None);

    assert_eq!(submitted.get("a"), Some(&None));
    assert!(!update_persisted_submission(&mut drafts, "a", None));
    assert!(!drafts[0].submitted);
    assert_eq!(drafts[0].session_path, None);
    assert!(submitted_draft_associations(&drafts).is_empty());
}

#[test]
fn background_submitted_draft_reconciles_while_b_stays_selected() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let submitted = HashMap::from([("a".into(), Some(path.clone()))]);
    let mut selected_draft = Some("b".to_owned());

    assert_eq!(
        reconciliation_candidates(&submitted, [path.as_path()].into_iter()),
        vec![("a".into(), path)]
    );
    clear_promoted_selection(&mut selected_draft, "a");
    assert_eq!(selected_draft.as_deref(), Some("b"));
}

#[test]
fn promotion_transfers_working_status_to_one_canonical_session_key() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let draft_key = draft_target("a");
    let session_key = session_target(&path);
    let mut statuses = HashMap::from([
        (draft_key.clone(), "Working".into()),
        (session_key.clone(), "Working".into()),
    ]);
    let mut completions = HashMap::new();

    transfer_draft_status(&mut statuses, &mut completions, "a", &path);

    assert_eq!(
        statuses.get(&session_key).map(String::as_str),
        Some("Working")
    );
    assert!(!statuses.contains_key(&draft_key));
    assert_eq!(statuses.len(), 1);
}

#[test]
fn reconciliation_requires_an_exact_discovered_path() {
    let path = PathBuf::from("/sessions/a.jsonl");
    let submitted = HashMap::from([
        ("a".into(), Some(path)),
        ("b".into(), Some(PathBuf::from("/sessions/b.jsonl"))),
    ]);

    assert!(
        reconciliation_candidates(
            &submitted,
            [std::path::Path::new("/sessions/other.jsonl")].into_iter(),
        )
        .is_empty()
    );
}

#[test]
fn materialized_codex_draft_can_enqueue_without_a_duplicate_client_key()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::app::infrastructure::persistence::StateStore;
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let draft = DraftSession::with_id(
        "codex-cli".into(),
        "codex-draft".into(),
        temp.path().to_owned(),
    );
    let id = store.allocate_app_session_id(&draft)?;
    let mut drafts = Vec::new();
    sync_materialized_draft(&mut drafts, &draft.id, id, temp.path(), "codex-cli", true);
    store.save_registry(&projects::Registry {
        projects: vec![temp.path().to_owned()],
        drafts,
        ..Default::default()
    })?;
    store.enqueue_prompt(
        &draft_target(&draft.id),
        "codex-cli",
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "fix this",
        &[],
    )?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    assert_eq!(store.load_registry()?.drafts[0].harness, "codex-cli");
    Ok(())
}
