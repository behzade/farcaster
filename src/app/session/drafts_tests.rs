use super::*;
use crate::agents::Backend;

#[gpui::test]
fn active_profile_uses_saved_native_session_identity_and_runtime_fallback(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::active_profile_uses_saved_native_session_identity_and_runtime_fallback"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let profile = "00000000-0000-4000-8000-000000000001";
                    let mut draft = DraftSession::fresh(Some(Backend::Pi), project.into());
                    draft.profile_id = Some(profile.into());
                    draft.submitted = true;
                    draft.session_path = Some(normalize_session_path(
                        &project.join("native-session.jsonl"),
                    ));
                    let mut store = crate::app::persistence::open().expect("store");
                    draft.app_session_id =
                        store.allocate_app_session_id(&draft).expect("bind draft");
                    app.sessions.all = store.cached_sessions("").expect("sessions").into();
                    app.sessions.selected_draft = None;
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    snapshot.selected_session = draft.session_path.clone();
                    snapshot.profile_id = Some("runtime-profile".into());
                    assert_eq!(app.active_profile_id().as_deref(), Some(profile));
                    app.sessions
                        .all
                        .iter_mut()
                        .find(|session| session.app_session_id == draft.app_session_id)
                        .expect("selected session")
                        .profile_id = None;
                    assert_eq!(
                        app.active_profile_id(),
                        None,
                        "a known built-in session owns its profile choice"
                    );
                    app.sessions.all.clear();
                    assert_eq!(app.active_profile_id().as_deref(), Some("runtime-profile"));

                    app.sessions.selected_draft = Some(draft.id.clone());
                    app.sessions.drafts.insert(0, draft);
                    assert_eq!(app.active_profile_id().as_deref(), Some(profile));
                    app.sessions.drafts[0].profile_id = None;
                    assert_eq!(
                        app.active_profile_id(),
                        None,
                        "a built-in draft owns its profile choice"
                    );
                });
            });
        },
    );
}

#[gpui::test]
fn draft_archive_sends_durable_intent_without_a_ui_locator(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::draft_archive_sends_durable_intent_without_a_ui_locator"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    futures::executor::block_on(app.sessions.writer.flush())
                        .expect("initial saves");
                    for bound in [false, true] {
                        for archived in [false, true] {
                            let mut draft = DraftSession::fresh(Some(Backend::Pi), project.into());
                            draft.submitted = true;
                            draft.archived = !archived;
                            draft.app_session_id =
                                super::super::draft_store::save(&draft).expect("save draft");
                            let mut store = crate::app::persistence::open().expect("store");
                            if bound {
                                let mut canonical = draft.clone();
                                canonical.session_path =
                                    Some(project.join(format!("{}.jsonl", draft.id)));
                                store
                                    .allocate_app_session_id(&canonical)
                                    .expect("runtime binding");
                            }
                            app.sessions.drafts.insert(0, draft.clone());
                            while runtime.try_recv_command().is_some() {}
                            app.request_draft_archive(draft.id.clone(), archived, window, cx);
                            assert_eq!(app.sessions.drafts[0].archived, archived);
                            let command = runtime.try_recv_command().expect("explicit intent");
                            let RuntimeCommand::SetSessionArchived {
                                request,
                                draft_saved: Some(saved),
                            } = command
                            else {
                                panic!("expected an archive with a draft save receipt");
                            };
                            saved
                                .recv_blocking()
                                .expect("receipt")
                                .expect("saved draft");
                            let crate::runtime::ArchiveTarget::App(app_session_id) = request.target
                            else {
                                panic!("archive must use durable identity")
                            };
                            let requested = request.archived;
                            assert_eq!(app_session_id.get(), draft.app_session_id);
                            assert_eq!(requested, archived);
                            let saved = store
                                .load_drafts()
                                .expect("drafts")
                                .into_iter()
                                .find(|saved| saved.id == draft.id)
                                .expect("saved draft");
                            assert_eq!(saved.archived, !archived);
                            store
                                .set_app_session_archived(app_session_id, requested)
                                .expect("apply intent");
                            let saved = store
                                .load_drafts()
                                .expect("drafts")
                                .into_iter()
                                .find(|saved| saved.id == draft.id)
                                .expect("saved draft");
                            assert_eq!(saved.archived, archived);
                            app.finish_archive(request, Ok(vec![]), cx);
                            assert!(!app.sessions.archive_writes.has_pending());
                        }
                    }
                });
            });
        },
    );
}

#[test]
fn startup_idle_preserves_only_the_unresolved_normal_submission() {
    use crate::{agents::PromptOutcome, protocol::PromptMode};

    let target = draft_target("starting");
    let path = PathBuf::from("/sessions/starting");
    let session_key = session_target(&path);
    let mut pending = HashMap::from([(
        "submission".into(),
        PendingSubmission {
            id: "submission".into(),
            submitted_at: std::time::Instant::now(),
            submitted_target: target.clone(),
            mode: PromptMode::Normal,
            text: "hello".into(),
            images: Vec::new(),
            pastes: Vec::new(),
            append_on_failure: false,
            result: None,
        },
    )]);
    let mut statuses = HashMap::from([(target.clone(), RunStatus::Working)]);
    let preserves = |target: &str, status: &RunStatus, pending: &_, statuses: &_| {
        preserve_submission_working_status(target, Some(&path), status, pending, statuses)
    };
    assert!(preserves(&target, &RunStatus::Done, &pending, &statuses));
    assert!(!preserves(
        "draft:other",
        &RunStatus::Done,
        &pending,
        &statuses
    ));
    assert!(!preserves(
        &session_key,
        &RunStatus::Done,
        &pending,
        &statuses
    ));
    for status in [&RunStatus::Failed, &RunStatus::Stopped] {
        assert!(!preserves(&target, status, &pending, &statuses));
    }

    transfer_draft_status(
        &mut statuses,
        &mut HashMap::new(),
        "starting",
        session_key.clone(),
    );
    pending
        .get_mut("submission")
        .expect("pending submission")
        .submitted_target = session_key.clone();
    assert!(preserves(&target, &RunStatus::Done, &pending, &statuses));
    assert!(preserves(
        &session_key,
        &RunStatus::Done,
        &pending,
        &statuses
    ));
    for mode in [PromptMode::Steer, PromptMode::FollowUp] {
        pending
            .get_mut("submission")
            .expect("pending submission")
            .mode = mode;
        assert!(!preserves(
            &session_key,
            &RunStatus::Done,
            &pending,
            &statuses
        ));
    }
    pending
        .get_mut("submission")
        .expect("pending submission")
        .mode = PromptMode::Normal;
    for terminal in [&RunStatus::Done, &RunStatus::Failed, &RunStatus::Stopped] {
        statuses.insert(session_key.clone(), *terminal);
        assert!(!preserves(&target, &RunStatus::Done, &pending, &statuses));
        assert!(!preserves(
            &session_key,
            &RunStatus::Done,
            &pending,
            &statuses
        ));
    }
    statuses.insert(session_key.clone(), RunStatus::Working);
    for outcome in [
        PromptOutcome::Accepted,
        PromptOutcome::RejectedBeforeAcceptance,
        PromptOutcome::DeliveryUnknown,
    ] {
        pending
            .get_mut("submission")
            .expect("pending submission")
            .result = Some((outcome, Some(path.clone())));
        assert!(!preserves(&target, &RunStatus::Done, &pending, &statuses));
        assert!(!preserves(
            &session_key,
            &RunStatus::Done,
            &pending,
            &statuses
        ));
    }
    pending.clear();
    assert!(!preserves(&target, &RunStatus::Done, &pending, &statuses));
}

#[test]
fn empty_startup_draft_stays_deleted_after_late_composer_save()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::app::infrastructure::persistence::{ComposerRecord, StateStore};

    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let project = temp.path().canonicalize()?;
    let mut store = StateStore::open_at(&database)?;
    let draft = DraftSession::with_id(Some(Backend::Pi), "startup".into(), project.clone());
    let id = store.allocate_app_session_id(&draft)?;
    let mut drafts = store.load_drafts()?;
    sync_materialized_draft(&mut drafts, "startup", id, &project, Some(Backend::Pi));
    store.remove_draft("startup")?;
    store.save_composer_session(&ComposerRecord {
        target: draft_target("startup"),
        ..Default::default()
    })?;
    drop(store);

    let reopened = StateStore::open_at(&database)?;
    assert!(reopened.load_drafts()?.is_empty());
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
fn drafts_materialize_once_and_survive_leaving_them() {
    let project = PathBuf::from("/project");
    let mut drafts = Vec::new();

    assert!(sync_materialized_draft(
        &mut drafts,
        "first",
        42,
        &project,
        Some(Backend::Codex),
    ));
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].id, "first");
    assert_eq!(drafts[0].app_session_id, 42);
    assert_eq!(drafts[0].harness, Some(Backend::Codex));
    assert!(!sync_materialized_draft(
        &mut drafts,
        "first",
        42,
        &project,
        Some(Backend::Codex),
    ));
    assert!(sync_materialized_draft(
        &mut drafts,
        "second",
        43,
        &project,
        Some(Backend::Codex),
    ));
    assert_eq!(drafts.len(), 2);
    assert!(drafts.iter().any(|draft| draft.id == "first"));
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
        harness: Some(Backend::Pi),
        profile_id: None,
        project: PathBuf::from("/project"),
        created_ms: 1,
        submitted: true,
        session_path: None,
        title: Some("Pending session".into()),
        archived: false,
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
        RunStatus::Working
    );
    assert_eq!(
        resolved_draft_status("b", &submitted, &HashMap::new()),
        RunStatus::Draft
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
    let mut statuses = HashMap::from([(session_key, RunStatus::NeedsInput)]);

    assert_eq!(
        resolved_draft_status("a", &submitted, &statuses),
        RunStatus::NeedsInput
    );
    statuses.insert(draft_target("a"), RunStatus::Failed);
    assert_eq!(
        resolved_draft_status("a", &submitted, &statuses),
        RunStatus::Failed
    );
    statuses.remove(&draft_target("a"));
    statuses.insert(session_target(&path), RunStatus::Done);
    assert_eq!(
        resolved_draft_status("a", &submitted, &statuses),
        RunStatus::Done
    );
    statuses.insert(session_target(&path), RunStatus::Working);
    assert_eq!(
        resolved_draft_status("a", &submitted, &statuses),
        RunStatus::Working
    );
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
        harness: Some(Backend::Pi),
        profile_id: None,
        project: project.clone(),
        created_ms: 1,
        submitted: false,
        session_path: None,
        title: None,
        archived: false,
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
        harness: Some(Backend::Pi),
        profile_id: None,
        project: PathBuf::from("/project"),
        created_ms: 1,
        submitted: false,
        session_path: None,
        title: None,
        archived: false,
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
        (draft_key.clone(), RunStatus::Working),
        (session_key.clone(), RunStatus::Working),
    ]);
    let mut completions = HashMap::new();

    transfer_draft_status(&mut statuses, &mut completions, "a", session_key.clone());

    assert_eq!(statuses.get(&session_key), Some(&RunStatus::Working));
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
        Some(Backend::Codex),
        "codex-draft".into(),
        temp.path().to_owned(),
    );
    let id = store.allocate_app_session_id(&draft)?;
    let mut drafts = Vec::new();
    sync_materialized_draft(
        &mut drafts,
        &draft.id,
        id,
        temp.path(),
        Some(Backend::Codex),
    );
    store.save_registry(&projects::Registry {
        projects: vec![temp.path().to_owned()],
        drafts,
        ..Default::default()
    })?;
    store.enqueue_prompt(
        &draft_target(&draft.id),
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "fix this",
        &[],
    )?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    assert_eq!(
        store.load_registry()?.drafts[0].harness,
        Some(Backend::Codex)
    );
    Ok(())
}

#[gpui::test]
fn untouched_drafts_prune_on_switch_and_quit(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::untouched_drafts_prune_on_switch_and_quit"
        ),
        cx,
        |cx, app, runtime, project| {
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let untouched = app.sessions.selected_draft.clone().expect("initial draft");
                    assert!(
                        crate::app::persistence::open()
                            .expect("store")
                            .load_drafts()
                            .expect("drafts")
                            .is_empty()
                    );
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    std::sync::Arc::make_mut(&mut snapshot.conversation)
                        .push_transport_error("old session".into());
                    app.sessions.preferred_harness = Some(Backend::Codex);
                    app.new_session(project.into(), window, cx);
                    assert!(
                        !app.sessions
                            .drafts
                            .iter()
                            .any(|draft| draft.id == untouched)
                    );
                });
            });
            cx.run_until_parked();
            let sequence = std::iter::from_fn(|| runtime.try_recv_command())
                .find_map(|command| match command {
                    RuntimeCommand::NewSession { request, .. } => request,
                    _ => None,
                })
                .expect("second draft dispatched");
            runtime.send_event(crate::runtime::RuntimeEvent::SelectionRouted(sequence));
            runtime.send_event(crate::runtime::RuntimeEvent::Snapshot {
                generation: 1,
                snapshot: std::sync::Arc::new(crate::runtime::RuntimeSnapshot {
                    project: project.into(),
                    harness: Some(Backend::Codex),
                    ..Default::default()
                }),
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert!(app.can_submit());
                    let engaged = app.sessions.selected_draft.clone().expect("next draft");
                    app.set_thinking_level(Some("high".into()), cx);
                    app.set_thinking_level(None, cx);
                    app.new_session(project.into(), window, cx);
                    assert!(app.sessions.drafts.iter().any(|draft| draft.id == engaged));
                    let last = app.sessions.selected_draft.clone().expect("last draft");
                    let target = draft_target(&last);
                    assert!(app.sync_drafts_before_quit(&target));
                    assert!(app.sessions.selected_draft.is_none());
                    assert!(!app.sessions.drafts.iter().any(|draft| draft.id == last));
                    assert!(
                        app.sync_drafts_before_quit(&target),
                        "quit callback must not recreate the draft"
                    );
                    futures::executor::block_on(app.sessions.writer.flush())
                        .expect("saved engagement");
                    let saved = crate::app::persistence::open()
                        .expect("store")
                        .load_drafts()
                        .expect("drafts");
                    assert_eq!(saved.len(), 1);
                    assert_eq!(saved[0].id, engaged);
                    assert!(saved[0].app_session_id > 0);
                });
            });
        },
    );
}

#[gpui::test]
fn user_actions_save_empty_drafts(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(module_path!(), "::user_actions_save_empty_drafts"),
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    // Trust remains off so terminal/editor entry points record the request
                    // without launching a native process in this test.
                    app.project.repository.execution_allowed = false;
                    let model: crate::protocol::Model = serde_json::from_value(serde_json::json!({
                        "id": "test", "name": "Test", "provider": "test"
                    }))
                    .expect("model");
                    for action in [
                        "harness",
                        "model",
                        "effort",
                        "tier",
                        "sandbox",
                        "terminal",
                        "editor",
                        "attachment",
                        "workgraph",
                        "folder",
                    ] {
                        app.new_session(project.into(), window, cx);
                        let id = app.sessions.selected_draft.clone().expect("draft");
                        // This offline fixture exercises actions on an already selected draft.
                        app.cancel_pending_session_switch();
                        match action {
                            "harness" => app.change_draft_harness(Backend::Pi, window, cx),
                            "model" => {
                                let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                                snapshot.harness = Some(Backend::Pi);
                                snapshot.access_mode = crate::runtime::HarnessAccessMode::Auto;
                                app.select_model_from_ui(&model, window, cx);
                            }
                            "effort" => app.set_thinking_level(Some("high".into()), cx),
                            "tier" => app.set_service_tier("fast".into(), cx),
                            "sandbox" => {
                                app.set_access_mode(crate::runtime::HarnessAccessMode::Full, cx)
                            }
                            "terminal" => {
                                app.activate_terminal_for_project(project.into(), window, cx)
                            }
                            "editor" => app.open_editor_request(
                                crate::app::workspace::editor::EditorRequest::Project(
                                    project.into(),
                                ),
                                window,
                                cx,
                            ),
                            "workgraph" => app.open_workgraph_surface(window, cx),
                            "folder" => {
                                app.sessions.folders.create("Work".into(), None);
                                let folder =
                                    app.sessions.folders.folders.last().expect("folder").id;
                                assert!(app.assign_session_folder(0, Some(folder), cx));
                                let session = app.sessions.draft_session_ids[&id];
                                assert_eq!(app.sessions.folders.folder_for(session), Some(folder));
                            }
                            "attachment" => {
                                let path = project.join("attachment.txt");
                                std::fs::write(&path, "attachment").expect("paste file");
                                app.composer.pastes.insert(
                                    draft_target(&id),
                                    vec![
                                        crate::app::composer::pastes::ComposerPaste::from_path(
                                            path,
                                        )
                                        .expect("paste"),
                                    ],
                                );
                                app.save_composer_attachments(&draft_target(&id));
                                app.remove_composer_paste(0, cx);
                            }
                            _ => unreachable!(),
                        }
                        app.project.pending_trust_command = None;
                        app.overlays.view.project_trust = false;
                        assert!(app.sync_drafts_before_quit(&draft_target(&id)), "{action}");
                        futures::executor::block_on(app.sessions.writer.flush())
                            .expect("engagement saved");
                        let saved = crate::app::persistence::open()
                            .expect("store")
                            .load_drafts()
                            .expect("drafts");
                        assert!(
                            saved
                                .iter()
                                .find(|draft| draft.id == id)
                                .expect(action)
                                .app_session_id
                                > 0,
                            "{action}"
                        );
                    }
                });
            });
        },
    );
}

#[gpui::test]
fn typing_then_clearing_a_draft_still_counts_as_engagement(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::typing_then_clearing_a_draft_still_counts_as_engagement"
        ),
        cx,
        |cx, app, _, project| {
            for value in [" ", ""] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.composer.input.update(cx, |input, cx| {
                            input.set_value(value, window, cx);
                            cx.emit(gpui_component::input::InputEvent::Change);
                        });
                    });
                });
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().expect("draft");
                    app.new_session(project.into(), window, cx);
                    futures::executor::block_on(app.sessions.writer.flush()).expect("saved");
                    assert!(
                        crate::app::persistence::open()
                            .expect("store")
                            .load_drafts()
                            .expect("drafts")
                            .iter()
                            .find(|draft| draft.id == id)
                            .expect("saved draft")
                            .app_session_id
                            > 0
                    );
                });
            });
        },
    );
}

#[gpui::test]
fn failed_first_engagement_save_keeps_the_draft_and_retries_before_quit(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::failed_first_engagement_save_keeps_the_draft_and_retries_before_quit"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().expect("draft");
                    let mut store = crate::app::persistence::open().expect("store");
                    store.with_connection(|connection| connection.execute_batch(
                        "CREATE TRIGGER reject_draft BEFORE INSERT ON sessions BEGIN SELECT RAISE(FAIL,'fixture draft save failure'); END;"
                    )).expect("reject writes");
                    app.set_thinking_level(Some("high".into()), cx);
                    app.set_thinking_level(None, cx);
                    assert!(!app.sync_drafts_before_quit(&draft_target(&id)));
                    assert_eq!(app.sessions.selected_draft.as_deref(), Some(id.as_str()));
                    assert!(app.sessions.pending_draft_saves.contains(&id));
                    assert!(store.load_drafts().expect("drafts").is_empty());
                    store.with_connection(|connection| connection.execute_batch("DROP TRIGGER reject_draft;")).expect("allow writes");
                    assert!(app.sync_drafts_before_quit(&draft_target(&id)));
                    assert!(!app.sessions.pending_draft_saves.contains(&id));
                    assert!(store.load_drafts().expect("drafts").iter().any(|draft| draft.id == id));
                });
            });
        },
    );
}

#[gpui::test]
fn cursor_selection_requires_sign_in_for_the_selected_account(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::cursor_selection_requires_sign_in_for_the_selected_account"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
                    let executable = home.join(".local/share/farcaster/cursor-sdk/1.0.35/bin/cursor-sdk-bridge");
                    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
                    std::fs::write(&executable, "fixture").unwrap();
                    let status = || crate::agents::backend_statuses().into_iter().find(|backend| backend.id == Backend::Cursor).unwrap();
                    assert!(status().sign_in_required);
                    assert!(!status().available);
                    assert!(app.editable_draft_harness().is_some());
                    let previous = app.sessions.preferred_harness;
                    app.change_draft_harness(Backend::Cursor, window, cx);
                    assert_eq!(app.sessions.preferred_harness, previous);
                    assert!(app.sessions.error.as_ref().unwrap().contains("Sign in"));
                    let credential = serde_json::json!({"version":1,"backendUrl":"https://api2.cursor.sh","apiKey":"fixture","createdAtMs":1}).to_string();
                    let save = |path: PathBuf| {
                        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                        std::fs::write(path, &credential).unwrap();
                    };
                    save(home.join(".cursor/sdk/auth.json"));
                    assert!(!status().sign_in_required);
                    assert!(status().available);
                    app.change_draft_harness(Backend::Cursor, window, cx);
                    assert_eq!(app.sessions.preferred_harness, Some(Backend::Cursor));
                    let id = "00000000-0000-4000-8000-000000000002";
                    app.settings.harness_profiles.replace(vec![crate::agents::HarnessProfile {
                        id:id.into(), name:"Other account".into(), backend:Backend::Cursor, executable, data_directory:None
                    }]).unwrap();
                    assert!(FarcasterApp::harness_sign_in_required(Backend::Cursor, Some(id)));
                    app.change_draft_harness_profile(Backend::Cursor, id.into(), window, cx);
                    assert_ne!(app.sessions.preferred_profile_id.as_deref(), Some(id));
                    save(crate::app::paths::data_dir().unwrap().join("session-locators/profiles").join(id).join("cursor-sdk-auth/auth.json"));
                    assert!(!FarcasterApp::harness_sign_in_required(Backend::Cursor, Some(id)));
                    app.change_draft_harness_profile(Backend::Cursor, id.into(), window, cx);
                    assert_eq!(app.sessions.preferred_profile_id.as_deref(), Some(id));
                    assert!(matches!(&app.project.pending_trust_command,
                        Some(RuntimeCommand::NewSession { profile_id: Some(profile), .. }) if profile == id));
                });
            });
        },
    );
}

#[gpui::test]
fn binding_an_idless_draft_preserves_the_committed_archive_state(cx: &mut gpui::TestAppContext) {
    use crate::app::persistence;
    use crate::runtime::ArchiveTarget;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::binding_an_idless_draft_preserves_the_committed_archive_state"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| app.update(cx, |app, cx| {
                let database = persistence::state_path().unwrap();
                for commit_before_binding in [false, true] {
                    let mut draft = DraftSession::fresh(Some(Backend::Pi), project.into());
                    draft.submitted = true;
                    let path = project.join(format!("{}.jsonl", draft.id));
                    app.sessions.drafts.insert(0, draft.clone());
                    while runtime.try_recv_command().is_some() {}
                    app.request_draft_archive(draft.id.clone(), true, window, cx);
                    assert!(app.sessions.drafts[0].archived);
                    let Some(RuntimeCommand::SetSessionArchived { mut request, draft_saved: Some(receipt) }) = runtime.try_recv_command() else { panic!("expected archive intent"); };
                    let id = if commit_before_binding {
                        let id = receipt.recv_blocking().unwrap().unwrap();
                        persistence::open().unwrap().archive_app_session(id,true).unwrap();
                        app.record_draft_submission(&draft_target(&draft.id), true, Some(path.clone()));
                        id
                    } else {
                        app.record_draft_submission(&draft_target(&draft.id), true, Some(path.clone()));
                        receipt.recv_blocking().unwrap().unwrap()
                    };
                    runtime.send_event(crate::runtime::RuntimeEvent::DraftIdentitySaved {
                        draft: draft.id.clone(),
                        app_session_id: id,
                    });
                    app.drain_runtime(cx);
                    futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                    let target = ArchiveTarget::App(id);
                    assert_eq!(app.sessions.archive_writes.confirmed(&target), Some(false));
                    assert_eq!(app.sessions.drafts[0].app_session_id, id.get());
                    request.target = target;
                    if commit_before_binding {
                        app.finish_archive(request, Ok(vec![]), cx);
                    } else {
                        let mut store = persistence::open().unwrap();
                        store.with_connection(|conn| conn.execute_batch("CREATE TRIGGER reject_archive BEFORE INSERT ON session_events
                            WHEN json_extract(NEW.body,'$.type')='session_archive_intent'
                            BEGIN SELECT RAISE(FAIL,'fixture archive failure'); END;")).unwrap();
                        let error = store.archive_app_session(id, true).unwrap_err();
                        assert!(error.contains("fixture archive failure"));
                        app.finish_archive(request, Err(error), cx);
                        store.with_connection(|conn| conn.execute_batch("DROP TRIGGER reject_archive")).unwrap();
                    }
                    assert_eq!(app.sessions.drafts[0].archived, commit_before_binding);
                    let mut reopened = persistence::StateStore::open_at(&database).unwrap();
                    let saved = reopened.load_drafts().unwrap().into_iter().find(|row| row.id == draft.id).unwrap();
                    assert_eq!(saved.archived, commit_before_binding);
                    assert_eq!(saved.session_path, Some(normalize_session_path(&path)));
                    let count: i64 = reopened.with_connection(|conn| conn.query_row(
                        "SELECT COUNT(*) FROM session_events WHERE session_id=?1 AND json_extract(body,'$.type')='session_archive_intent'",
                        [id.get()], |row| row.get(0))).unwrap();
                    assert_eq!(count, if commit_before_binding {1} else {0});
                }
            }));
        },
    );
}

#[gpui::test]
fn working_draft_archive_requires_confirmation_and_targets_its_actor(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::working_draft_archive_requires_confirmation_and_targets_its_actor"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let mut draft = DraftSession::fresh(Some(Backend::Pi), project.into());
                    draft.submitted = true;
                    app.activity
                        .run_statuses
                        .insert(draft_target(&draft.id), RunStatus::Working);
                    app.sessions.drafts.insert(0, draft.clone());
                    while runtime.try_recv_command().is_some() {}
                    for confirm in [false, true] {
                        app.request_draft_archive(draft.id.clone(), true, window, cx);
                        assert!(app.sessions.pending_archive.is_some());
                        assert!(!app.sessions.drafts[0].archived);
                        assert!(runtime.try_recv_command().is_none());
                        if confirm {
                            app.stop_and_archive_pending_session(window, cx);
                            let Some(RuntimeCommand::StopAndArchiveDraft {
                                draft: key,
                                identity: crate::runtime::DraftArchiveIdentity::Pending(saved),
                            }) = runtime.try_recv_command()
                            else {
                                panic!("missing draft stop");
                            };
                            assert_eq!(key, draft.id);
                            saved.recv_blocking().unwrap().unwrap();
                        } else {
                            app.close_archive_confirmation(window, cx);
                        }
                        assert!(runtime.try_recv_command().is_none());
                    }
                })
            });
        },
    );
}

#[gpui::test]
fn archive_during_first_send_targets_only_the_pending_draft(cx: &mut gpui::TestAppContext) {
    use crate::{agents::PromptOutcome, protocol::PromptMode, runtime::RuntimeEvent};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::archive_during_first_send_targets_only_the_pending_draft"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().expect("selected draft");
                    let sibling = DraftSession::fresh(Some(Backend::Codex), project.into());
                    let sibling_id = sibling.id.clone();
                    app.sessions.drafts.insert(0, sibling);
                    std::sync::Arc::make_mut(&mut app.snapshot).harness = Some(Backend::Codex);

                    for reject in [true, false] {
                        app.submit("first send".into(), PromptMode::Normal, window, cx);
                        let submission = std::iter::from_fn(|| runtime.try_recv_command())
                            .find_map(|command| match command {
                                RuntimeCommand::Prompt {
                                    submission_id,
                                    target,
                                    ..
                                } if target == draft_target(&id) => Some(submission_id),
                                _ => None,
                            })
                            .expect("first send awaiting a receipt");
                        if !reject {
                            app.sessions.selected_draft = Some(sibling_id.clone());
                        }
                        app.request_draft_archive(sibling_id.clone(), true, window, cx);
                        assert!(app.sessions.pending_archive.is_none());
                        app.request_draft_archive(id.clone(), true, window, cx);
                        assert!(app.sessions.pending_archive.is_some());
                        if reject {
                            runtime.send_event(RuntimeEvent::PromptResult {
                                submission_id: Some(submission),
                                target: draft_target(&id),
                                outcome: PromptOutcome::RejectedBeforeAcceptance,
                                session: None,
                            });
                            app.drain_runtime(cx);
                            app.resolve_pending_submission(window, cx);
                        }
                        app.stop_and_archive_pending_session(window, cx);
                        let stop =
                            std::iter::from_fn(|| runtime.try_recv_command()).find_map(|command| {
                                match command {
                                    RuntimeCommand::StopAndArchiveDraft { draft, identity } => {
                                        Some((draft, identity))
                                    }
                                    _ => None,
                                }
                            });
                        if reject {
                            assert!(stop.is_none());
                            continue;
                        }
                        let (target, crate::runtime::DraftArchiveIdentity::Pending(saved)) =
                            stop.expect("confirmed draft stop")
                        else {
                            panic!("expected a draft identity receipt");
                        };
                        assert_eq!(target, id);
                        let identity = saved.recv_blocking().unwrap().unwrap();
                        let row = crate::app::persistence::open()
                            .unwrap()
                            .load_drafts()
                            .unwrap()
                            .into_iter()
                            .find(|draft| draft.app_session_id == identity.get())
                            .expect("archive identity belongs to a saved draft");
                        assert_eq!(row.id, id);
                        assert!(!row.submitted);
                        assert!(row.title.is_none());
                    }
                });
            });
        },
    );
}

#[gpui::test]
fn submitting_draft_waits_when_only_a_sibling_has_arrived(cx: &mut gpui::TestAppContext) {
    use crate::{agent_activity::SessionStatusRecord, runtime::RuntimeEvent};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::submitting_draft_waits_when_only_a_sibling_has_arrived"
        ),
        cx,
        |cx, app, runtime, project| {
            let path = normalize_session_path(&project.join("shared"));
            let sibling = crate::sessions::SessionSummary::from_cached(
                "sibling".into(),
                path.clone(),
                project.into(),
                "Sibling".into(),
                String::new(),
                String::new(),
                None,
                std::time::UNIX_EPOCH,
                0,
                Default::default(),
                false,
                false,
                String::new(),
            )
            .with_app_session_id(51);
            let mut owner = sibling.clone().with_app_session_id(52);
            owner.harness = Backend::Codex;
            owner.id = "owner".into();
            let owner_buffer = crate::sessions::session_buffer(owner.harness, &path);
            let sibling_buffer = crate::sessions::session_buffer(sibling.harness, &path);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let draft = DraftSession::fresh(Some(owner.harness), project.into());
                    let id = draft.id.clone();
                    let key = draft_target(&id);
                    app.switch_composer_target(key.clone(), window, cx);
                    app.sessions.selected_draft = Some(id.clone());
                    app.sessions.drafts.push(draft);
                    assert!(app.sessions.all.by_path(&path).is_none());
                    std::sync::Arc::make_mut(&mut app.snapshot).harness = Some(owner.harness);
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("owner history", window, cx));
                    app.submit(
                        "owner history".into(),
                        crate::protocol::PromptMode::Normal,
                        window,
                        cx,
                    );
                    let submission = std::iter::from_fn(|| runtime.try_recv_command())
                        .find_map(|command| {
                            if let RuntimeCommand::Prompt {
                                target,
                                submission_id,
                                ..
                            } = command
                            {
                                assert_eq!(target, key);
                                Some(submission_id)
                            } else {
                                None
                            }
                        })
                        .expect("draft submitted");
                    use crate::storage::{PromptSubmission, SessionRepository as _};
                    let queued = crate::app::persistence::shared()
                        .unwrap()
                        .submit_prompt(PromptSubmission {
                            target: &key,
                            harness: owner.harness,
                            project,
                            session: None,
                            profile_id: None,
                            provisional_title: Some("owner history"),
                            mode: crate::protocol::PromptMode::Normal,
                            message: "owner history",
                            display_message: None,
                            invocation: None,
                            images: &[],
                        })
                        .unwrap();
                    runtime.send_event(RuntimeEvent::DraftPromptCommitted {
                        provisional_title: Some("owner history".into()),
                        draft: id.clone(),
                        app_session_id: queued.app_session_id,
                    });
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("owner input", window, cx));
                    let native = SessionStatusRecord::Native(owner.activity_key());
                    runtime.send_event(RuntimeEvent::SessionStatus {
                        record: Some(native.clone()),
                        target: key.clone(),
                        session: Some(path.clone()),
                        status: RunStatus::Working,
                    });
                    app.drain_runtime(cx);
                    assert_eq!(
                        app.activity.run_statuses.get(&native.status_key()),
                        Some(&RunStatus::Working)
                    );
                    assert_eq!(
                        app.activity.run_statuses.get(&key),
                        Some(&RunStatus::Working)
                    );
                    assert!(
                        !app.activity
                            .run_statuses
                            .contains_key(&session_target(&path))
                    );

                    runtime.send_event(RuntimeEvent::SessionUpdated(sibling.clone()));
                    app.drain_runtime(cx);
                    let visible = app.sessions.visible.by_path(&path).unwrap();
                    assert!(visible.same_record(&sibling));
                    assert_eq!(app.rail_row_status(visible, false), RunStatus::Done);

                    runtime.send_event(RuntimeEvent::PromptResult {
                        submission_id: Some(submission),
                        target: key.clone(),
                        outcome: crate::agents::PromptOutcome::Accepted,
                        session: Some(path.clone()),
                    });
                    app.drain_runtime(cx);
                    assert_eq!(app.sessions.submitted_drafts[&id].as_ref(), Some(&path));
                    assert_eq!(
                        app.activity.run_statuses.get(&key),
                        Some(&RunStatus::Working)
                    );
                    assert!(
                        !app.activity
                            .run_statuses
                            .contains_key(&session_target(&path))
                    );
                    assert_eq!(app.rail_row_status(&sibling, false), RunStatus::Done);
                    runtime.send_event(RuntimeEvent::SessionStatus {
                        record: None,
                        target: key.clone(),
                        session: Some(path.clone()),
                        status: RunStatus::Working,
                    });
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: app.runtime_generation,
                        snapshot: std::sync::Arc::new(crate::runtime::RuntimeSnapshot {
                            harness: Some(owner.harness),
                            project: project.into(),
                            selected_session: Some(path.clone()),
                            live_session: Some(path.clone()),
                            live_status: RunStatus::Working,
                            ..Default::default()
                        }),
                    });
                    app.drain_runtime(cx);
                    assert_eq!(app.snapshot.live_session.as_ref(), Some(&path));
                    let draft = app
                        .sessions
                        .drafts
                        .iter()
                        .find(|draft| draft.id == id)
                        .unwrap();
                    assert_eq!(app.draft_rail_status(draft), RunStatus::Working);
                    assert_eq!(
                        app.activity.run_statuses.get(&key),
                        Some(&RunStatus::Working)
                    );
                    assert_eq!(app.rail_row_status(&sibling, false), RunStatus::Done);
                    assert!(app.bound_draft_buffer(&id, &path).is_none());
                    assert!(!app.reconcile_submitted_drafts(cx));
                    assert_eq!(app.composer.sessions.current_buffer(), key);
                    assert_eq!(app.sessions.selected_draft.as_ref(), Some(&id));
                    app.sessions.all = vec![sibling.clone(), owner.clone()].into();
                    assert_eq!(
                        app.bound_draft_buffer(&id, &path).as_deref(),
                        Some(owner_buffer.as_str())
                    );
                    assert!(
                        !app.reconcile_submitted_drafts(cx),
                        "only the sibling is visible"
                    );
                    app.sessions.all = vec![sibling.clone()].into();
                    let draft = app
                        .sessions
                        .drafts
                        .iter_mut()
                        .find(|row| row.id == id)
                        .unwrap();
                    draft.harness = None;
                    assert!(draft.app_session_id > 0);
                    owner.app_session_id = draft.app_session_id;
                    assert!(app.bound_draft_buffer(&id, &path).is_none());
                    runtime.send_event(RuntimeEvent::SessionUpdated(owner.clone()));
                    app.drain_runtime(cx);
                    assert_eq!(app.rail_row_status(&owner, false), RunStatus::Working);
                    assert_eq!(app.rail_row_status(&sibling, false), RunStatus::Done);
                    assert_eq!(app.composer.sessions.current_buffer(), owner_buffer);
                    assert_eq!(app.composer.sessions.current().text, "owner input");
                    assert!(
                        app.composer
                            .sessions
                            .snapshot_for(&sibling_buffer)
                            .text
                            .is_empty()
                    );
                    assert_eq!(
                        app.composer
                            .sessions
                            .previous_history(Default::default())
                            .unwrap()
                            .text,
                        "owner history"
                    );
                    assert!(!app.sessions.submitted_drafts.contains_key(&id));
                })
            });
        },
    );
}
