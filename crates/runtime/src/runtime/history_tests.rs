use super::*;
use crate::agents::Backend;

const ONE_PIXEL_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

#[test]
fn stopped_session_context_survives_history_selection_and_reopening() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().to_owned();
    let path = project.join("session.jsonl");
    let state =
        farcaster_storage::SharedStateStore::open_at(&project.join("state.sqlite3")).unwrap();
    let metrics = json!({"contextUsage":{"tokens":120,"contextWindow":1000,"percent":12.0}});
    let (mut owner, _) = crate::runtime::tests::owner_without_process(project.clone());
    owner.state = Some(state.clone());
    owner.active_session = Some(path.clone());
    owner.snapshot.selected_session = Some(path.clone());
    owner.snapshot.stats = metrics.clone();
    owner.snapshot.document_ready = true;
    owner.snapshot.session_goal = Some(agents::SessionGoal {
        objective: "Fix preview".into(),
        status: "active".into(),
        token_budget: Some(900),
        tokens_used: 80,
        time_used_seconds: 10,
    });
    owner.snapshot.last_outcome = Some(SessionOutcome::Failed);
    owner.snapshot.observed_service_tier = Some("priority".into());
    conversation_mut(&mut owner.snapshot).restore_cache_usage(crate::conversation::CacheUsage {
        read_tokens: 100,
        prompt_tokens: 200,
    });
    let facts = owner.snapshot.session_facts();
    owner.publish_session_metadata();
    drop(owner);
    let (mut restored, _) = crate::runtime::tests::owner_without_process(project.clone());
    restored.state = Some(state);
    assert!(restored.bind_history_selection(path.clone(), project.clone()));
    assert_eq!(restored.snapshot.stats, metrics);
    for (kind, usage) in [
        (HistoryLoadKind::Selection, Value::Null),
        (HistoryLoadKind::DocumentRefresh, json!({"totalTokens":99})),
    ] {
        restored.apply_history(HistoryResult {
            transcript_stamp: None,
            generation: restored.history_generation,
            path: path.clone(),
            project: project.clone(),
            kind,
            result: Ok(LoadedHistory {
                messages: vec![json!({
                    "role":"assistant", "content":[{"type":"text","text":"done"}], "usage":usage
                })]
                .into(),
                model: None,
                thinking_level: None,
                pending_question: None,
                prompt_deliveries: None,
            }),
        });
        assert_eq!(restored.snapshot.stats, metrics);
        assert_eq!(restored.snapshot.session_goal, facts.goal);
        assert_eq!(semantic_status(&restored.snapshot), RunStatus::Failed);
        assert_eq!(restored.snapshot.selected_service_tier(), Some("priority"));
        assert_eq!(
            restored.snapshot.conversation.cache_usage(),
            facts.cache_usage
        );
        assert!(restored.snapshot.document_ready);
        assert_eq!(restored.snapshot.conversation.items[0].text, "done");
        assert!(!restored.snapshot.connected);
        assert!(!restored.snapshot.conversation.running);
    }
}

#[test]
fn background_refresh_updates_a_retired_document_without_losing_metrics() {
    let project = PathBuf::from("/project");
    let path = PathBuf::from("/sessions/retired.jsonl");
    let (mut owner, events) = crate::runtime::tests::owner_without_process(project.clone());
    owner.active_session = Some(path.clone());
    owner.idle_retirement.retired = true;
    owner.snapshot.harness = owner.harness;
    owner.snapshot.connected = false;
    owner.snapshot.document_ready = true;
    owner.snapshot.selected_session = Some(path.clone());
    owner.snapshot.stats = json!({
        "tokens": {"input": 100, "output": 20, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 120},
        "contextUsage": {"tokens": 120, "contextWindow": 1000, "percent": 12.0}
    });
    conversation_mut(&mut owner.snapshot).restore_cache_usage(crate::conversation::CacheUsage {
        read_tokens: 30,
        prompt_tokens: 100,
    });
    let previous = owner.snapshot.clone();
    owner.apply_history(HistoryResult {
        transcript_stamp: None,
        generation: owner.history_generation,
        path: path.clone(),
        project,
        kind: HistoryLoadKind::DocumentRefresh,
        result: Ok(LoadedHistory {
            messages: vec![
                json!({"role": "assistant", "content": [{"type": "text", "text": "new history"}]}),
            ]
            .into(),
            model: None,
            thinking_level: None,
            pending_question: None,
            prompt_deliveries: None,
        }),
    });
    assert_eq!(owner.snapshot.conversation.items[0].text, "new history");
    assert_eq!(owner.snapshot.stats, previous.stats);
    assert_eq!(
        owner.snapshot.conversation.average_cache_hit_rate,
        Some(30.0)
    );
    assert!(owner.snapshot.document_ready);
    assert!(!owner.snapshot.connected);
    assert!(!owner.snapshot.history_preview);
    assert!(owner.parked_snapshot.is_none());
    assert_eq!(owner.active_session, Some(path));
    assert!(
        !events
            .try_iter()
            .any(|event| matches!(event, RuntimeEvent::HistoryReset { .. }))
    );
}

#[test]
fn history_without_local_annotations_keeps_shared_messages() {
    let temp = tempfile::tempdir().expect("fixture");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3")).expect("store");
    let original = Arc::new(vec![json!({"role":"assistant","content":"answer"})]);
    let mut messages = original.clone();
    annotate_history_presentations(Some(&store), &temp.path().join("session"), &mut messages);
    assert!(Arc::ptr_eq(&messages, &original));
}

#[test]
fn refresh_history_preserves_config_and_selects_profile_from_locator() {
    let temp = tempfile::tempdir().expect("fixture");
    let session_id = uuid::Uuid::new_v4().to_string();
    let profile_id = uuid::Uuid::new_v4().to_string();
    let root = temp.path().join("custom-claude");
    let project = root.join("projects/fixture");
    std::fs::create_dir_all(&project).expect("Claude project directory");
    let row = json!({
        "type": "user", "uuid": "u", "parentUuid": null,
        "message": {"role": "user", "content": "custom profile history"}
    });
    std::fs::write(
        project.join(format!("{session_id}.jsonl")),
        format!("{row}\n"),
    )
    .expect("Claude history");
    let locator = temp
        .path()
        .join("profiles")
        .join(&profile_id)
        .join("claude")
        .join(session_id);
    let (mut owner, _events) = crate::runtime::tests::owner_without_process(temp.path().into());
    owner.harness = Some(Backend::Claude);
    owner
        .process_command
        .profiles
        .replace(vec![agents::HarnessProfile {
            id: profile_id.clone(),
            name: "Custom Claude".into(),
            backend: Backend::Claude,
            executable: PathBuf::from("claude"),
            data_directory: Some(root),
        }])
        .expect("saved profile");
    let active_profile = uuid::Uuid::new_v4().to_string();
    owner.process_command.profile_id = Some(active_profile.clone());
    let (sender, receiver) = std::sync::mpsc::channel();
    owner.history_tx = sender;
    for kind in [HistoryLoadKind::Selection, HistoryLoadKind::DocumentRefresh] {
        owner.refresh_history(locator.clone(), temp.path().into(), kind);
        let result = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("history worker result");
        let HistoryUpdate::Finished(result) = result else {
            panic!("expected complete Claude history")
        };
        assert_eq!(result.path, locator);
        assert_eq!(result.project, temp.path());
        let history = result
            .result
            .as_ref()
            .expect("profile-aware runtime history");
        assert_eq!(history.messages.len(), 1);
        assert_eq!(
            history.messages[0]["content"][0]["text"],
            "custom profile history"
        );
        assert_eq!(
            owner.process_command.profile_id.as_deref(),
            Some(active_profile.as_str())
        );
        owner.apply_history(result);
        assert_eq!(
            owner.snapshot.profile_id.as_deref(),
            Some(profile_id.as_str())
        );
    }
}

#[test]
fn dismissed_queue_row_does_not_return_on_history_refresh_or_reopen() -> Result<(), String> {
    use crate::runtime::tests::owner_without_process;
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut state = StateStore::open_at(&database)?;
    let id = state.enqueue_prompt(
        "draft:cancel",
        Backend::Codex,
        temp.path(),
        None,
        PromptMode::FollowUp,
        "stale",
        &[],
    )?;
    state.record_prompt_acceptance(id, "draft:cancel", Some(&session), "stale-id", true)?;
    let (mut owner, _) = owner_without_process(temp.path().into());
    owner.state = Some(state.into());
    owner.harness = Some(Backend::Codex);
    owner.snapshot.project = temp.path().into();
    owner.snapshot.selected_session = Some(session.clone());
    owner.snapshot.history_preview = true;
    let mut history = Arc::new(vec![]);
    owner.state.as_ref().unwrap().with(|store| {
        annotate_history_presentations(Some(store), &session, &mut history);
        Ok(())
    })?;
    conversation_mut(&mut owner.snapshot).replace_history(&history);
    assert_eq!(owner.snapshot.conversation.pending_receipts().len(), 1);
    owner.apply_command(RuntimeCommand::DismissReceipt {
        session: temp.path().join("wrong"),
        id: "stale-id".into(),
    });
    assert_eq!(owner.snapshot.conversation.pending_receipts().len(), 1);
    owner.apply_command(RuntimeCommand::DismissReceipt {
        session: session.clone(),
        id: "stale-id".into(),
    });
    assert!(owner.snapshot.conversation.pending_receipts().is_empty());
    drop(owner);
    let state = StateStore::open_at(&database)?;
    let mut history = Arc::new(vec![]);
    annotate_history_presentations(Some(&state), &session, &mut history);
    assert!(history.is_empty());
    Ok(())
}

#[test]
fn authoritative_delivery_evidence_resolves_saved_receipts_by_exact_id()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::protocol::PromptMode;
    use crate::runtime::tests::owner_without_process;

    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    for submission_id in ["receipt:delivered", "receipt:pending", "receipt:missing"] {
        let outbox_id = store.enqueue_prompt(
            "draft:reconcile",
            Backend::Codex,
            temp.path(),
            None,
            PromptMode::Steer,
            submission_id,
            &[],
        )?;
        store.record_prompt_acceptance(
            outbox_id,
            "draft:reconcile",
            Some(&session),
            submission_id,
            true,
        )?;
    }

    let (mut owner, _events) = owner_without_process(temp.path().to_owned());
    owner.harness = Some(Backend::Codex);
    owner.state = Some(store.into());
    owner.active_session = Some(session.clone());
    owner.apply_response(crate::agents::SessionResponse::success(
        None,
        crate::agents::SessionResponsePayload::LoadHistory(
            crate::agents::SessionHistory::Replace {
                messages: vec![json!({
                    "role": "user",
                    "content": [{"type":"text", "text":"receipt:delivered"}],
                    "submissionId": "receipt:delivered",
                    "deliveryStatus": "delivered",
                })],
                prompt_deliveries: Some(crate::sessions::PromptDeliveryReconciliation {
                    absence_is_not_delivered: true,
                    delivered: vec!["receipt:delivered".into()],
                    pending: vec!["receipt:pending".into()],
                }),
            },
        ),
    ));

    let pending = owner.snapshot.conversation.pending_receipts();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "receipt:pending");
    drop(owner);
    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store
            .accepted_prompt_history(&session)?
            .iter()
            .filter_map(|message| message.get("submissionId").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        ["receipt:pending"]
    );
    Ok(())
}

#[test]
fn accepted_image_only_prompt_survives_empty_backend_history_and_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let image = crate::protocol::PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into());
    let mut store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        "draft:image",
        Backend::Pi,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "",
        std::slice::from_ref(&image),
    )?;
    store.record_prompt_acceptance(id, "draft:image", Some(&session), "image-receipt", false)?;
    store.record_prompt_acceptance(id, "draft:image", Some(&session), "image-receipt", false)?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    assert_eq!(store.queued_prompts()?.len(), 1);
    let mut messages = Arc::new(Vec::new());
    annotate_history_presentations(Some(&store), &session, &mut messages);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["submissionId"], "image-receipt");
    assert_eq!(messages[0]["deliveryStatus"], "accepted");
    assert_eq!(
        messages[0]["content"][1],
        serde_json::json!({"type":"image", "data":image.data, "mimeType":image.mime_type})
    );
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&messages);
    assert_eq!(conversation.items.len(), 1);
    assert_eq!(conversation.items[0].text, "");
    assert_eq!(conversation.items[0].images.len(), 1);
    let mut backend = Arc::new(vec![serde_json::json!({
        "role":"assistant",
        "content":[{"type":"text", "text":"existing"}],
    })]);
    annotate_history_presentations(Some(&store), &session, &mut backend);
    assert_eq!(backend.len(), 1);
    assert!(
        store
            .accepted_prompt_history(&temp.path().join("other"))?
            .is_empty()
    );
    Ok(())
}

#[test]
fn uncorrelated_normal_is_not_duplicated_and_correlated_normal_survives_old_history()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    let first = store.enqueue_prompt(
        "draft:first",
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "first",
        &[],
    )?;
    let second = store.enqueue_prompt(
        "draft:second",
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "second",
        &[crate::protocol::PromptImage::new(
            ONE_PIXEL_PNG.into(),
            "image/png".into(),
        )],
    )?;
    store.record_prompt_acceptance(first, "draft:first", Some(&session), "receipt:first", false)?;
    store.record_prompt_acceptance(
        second,
        "draft:second",
        Some(&session),
        "receipt:second",
        true,
    )?;
    let delivered = store.enqueue_prompt(
        "draft:delivered",
        Backend::Codex,
        temp.path(),
        None,
        crate::protocol::PromptMode::Normal,
        "already in history",
        &[],
    )?;
    store.record_prompt_receipt_delivered("receipt:delivered", Some(delivered))?;
    store.record_prompt_acceptance(
        delivered,
        "draft:delivered",
        Some(&session),
        "receipt:delivered",
        true,
    )?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    let mut history = Arc::new(vec![serde_json::json!({
        "role":"user",
        "content":[{"type":"text", "text":"first"}],
    })]);
    assert_eq!(
        store.accepted_prompt_history(&session)?.len(),
        2,
        "only the receipt with actual delivery evidence is excluded at storage"
    );
    annotate_history_presentations(Some(&store), &session, &mut history);
    assert_eq!(history.len(), 2);
    assert!(history[0].get("submissionId").is_none());
    assert_eq!(history[1]["submissionId"], "receipt:second");
    assert_eq!(history[1]["content"][0]["text"], "second");
    assert_eq!(
        history[1]["content"][1],
        serde_json::json!({"type":"image", "data":ONE_PIXEL_PNG, "mimeType":"image/png"})
    );
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&history);
    assert_eq!(conversation.items.len(), 2);
    assert_eq!(conversation.items[1].text, "second");
    assert_eq!(conversation.items[1].images.len(), 1);
    Ok(())
}

#[test]
fn cold_history_restores_queued_receipt_identity_without_claiming_delivery()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::protocol::{PromptImage, PromptMode};
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    for (id, mode, tracked) in [
        ("queued-steer", PromptMode::Steer, true),
        ("queued-follow", PromptMode::FollowUp, false),
    ] {
        let row = store.enqueue_prompt(
            "draft:pending",
            Backend::Codex,
            temp.path(),
            None,
            mode,
            "same text",
            &[PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into())],
        )?;
        store.record_prompt_acceptance(row, "draft:pending", Some(&session), id, tracked)?;
    }
    drop(store);
    let store = StateStore::open_at(&database)?;
    let mut history = Arc::new(vec![
        json!({"role":"assistant", "content":[{"type":"text", "text":"older answer"}]}),
    ]);
    annotate_history_presentations(Some(&store), &session, &mut history);
    assert_eq!(
        history.len(),
        2,
        "only tracked queued receipts stay pending once native history exists"
    );
    assert_eq!(history[1]["submissionId"], "queued-steer");
    let mut conversation = ConversationState::default();
    conversation.replace_history(&history);
    assert_eq!(
        conversation.items.len(),
        1,
        "only the older answer reached the model"
    );
    let pending = conversation.pending_receipts();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "queued-steer");
    assert_eq!(pending[0].mode, Some(PromptMode::Steer));
    assert_eq!(pending[0].images.len(), 1);
    assert!(!pending[0].unknown);
    assert!(
        conversation.queue.steering.is_empty() && conversation.queue.follow_up.is_empty(),
        "saved receipts are presentation, not executable input"
    );
    assert_eq!(
        store.queued_prompts()?.len(),
        2,
        "admission is not delivery"
    );
    for receipt in &history[1..] {
        assert_eq!(receipt["content"][1]["data"], ONE_PIXEL_PNG);
        let message = serde_json::from_value(receipt.clone()).expect("receipt message");
        conversation.record_prompt_delivery(
            receipt["submissionId"]
                .as_str()
                .expect("receipt submission ID"),
            Some(&message),
            agents::DeliveryStatus::Delivered,
        );
    }
    assert_eq!(conversation.items.len(), 2);
    assert!(conversation.pending_receipts().is_empty());
    assert_eq!(conversation.items[1].text, "same text");
    assert_eq!(conversation.items[1].images.len(), 1);
    Ok(())
}

#[test]
fn untracked_queued_receipts_stay_pending_only_when_native_history_is_missing()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::protocol::PromptMode;
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session");
    let mut store = StateStore::open_at(&database)?;
    for (id, text) in [
        ("steer-opencode", "it's an opencode session"),
        (
            "steer-model",
            "I changed the model in between, messages currently shown are from openai astra, later ones are from deepseek flash 4.1",
        ),
    ] {
        let row = store.enqueue_prompt(
            "draft:pi",
            Backend::Pi,
            temp.path(),
            None,
            PromptMode::Steer,
            text,
            &[],
        )?;
        store.record_prompt_acceptance(row, "draft:pi", Some(&session), id, false)?;
    }
    drop(store);
    let store = StateStore::open_at(&database)?;

    let mut empty = Arc::new(Vec::new());
    annotate_history_presentations(Some(&store), &session, &mut empty);
    assert_eq!(empty.len(), 2);
    let mut conversation = ConversationState::default();
    conversation.replace_history(&empty);
    assert_eq!(conversation.pending_receipts().len(), 2);
    assert!(conversation.items.is_empty());

    let mut history = Arc::new(vec![
        json!({"role":"user", "content":[{"type":"text", "text":"it's an opencode session"}]}),
        json!({"role":"user", "content":[{"type":"text", "text":"I changed the model in between, messages currently shown are from openai astra, later ones are from deepseek flash 4.1"}]}),
    ]);
    annotate_history_presentations(Some(&store), &session, &mut history);
    assert_eq!(history.len(), 2);
    assert!(
        history
            .iter()
            .all(|message| message.get("submissionId").is_none())
    );
    let mut conversation = ConversationState::default();
    conversation.replace_history(&history);
    assert!(conversation.pending_receipts().is_empty());
    assert_eq!(conversation.items.len(), 2);
    assert_eq!(conversation.items[0].text, "it's an opencode session");
    Ok(())
}

#[test]
fn reopened_accepted_queue_receipts_stay_off_transcript_until_delivery()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::protocol::{PromptImage, PromptMode};
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("native-session");
    let image = PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into());
    let mut store = StateStore::open_at(&database)?;
    for (id, mode) in [
        ("steer", PromptMode::Steer),
        ("follow", PromptMode::FollowUp),
    ] {
        let row = store.enqueue_prompt(
            "draft:queue",
            Backend::Codex,
            temp.path(),
            None,
            mode,
            "same text",
            std::slice::from_ref(&image),
        )?;
        store.record_prompt_acceptance(row, "draft:queue", Some(&session), id, true)?;
    }
    drop(store);
    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store.queued_prompts()?.len(),
        2,
        "admission is not delivery"
    );
    let receipts = store.accepted_prompt_history(&session)?;
    assert_eq!(receipts.len(), 2);
    for receipt in &receipts {
        assert_eq!(receipt["queued"], true);
        assert_eq!(receipt["content"][1]["data"], ONE_PIXEL_PNG);
    }
    let mut state = ConversationState::default();
    state.replace_history(&receipts);
    assert!(
        state.items.is_empty(),
        "reopen cannot turn receipt into delivery"
    );
    for receipt in &receipts {
        let id = receipt["submissionId"]
            .as_str()
            .expect("receipt submission ID");
        let message = serde_json::from_value(receipt.clone()).expect("receipt message");
        state.record_prompt_delivery(id, Some(&message), agents::DeliveryStatus::Delivered);
        state.record_prompt_delivery(id, Some(&message), agents::DeliveryStatus::Delivered);
    }
    assert_eq!(state.items.len(), 2);
    assert!(
        state
            .items
            .iter()
            .all(|item| item.text == "same text" && item.images.len() == 1)
    );
    Ok(())
}

#[test]
fn reselect_keeps_in_flight_history_and_displays_load_failure() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().to_path_buf();
    let path = project.join(Backend::Codex.as_str()).join("native-id");
    let (mut owner, _) = crate::runtime::tests::owner_without_process(project.clone());
    owner.harness = Some(Backend::Codex);
    owner.session_id = Some(path.to_string_lossy().into_owned());
    assert!(owner.bind_history_selection(path.clone(), project.clone()));
    assert_eq!(owner.session_id.as_deref(), Some("native-id"));
    owner.history_generation = 2;
    owner.history_selection_generation = Some(2);
    let tail = |generation, text| HistoryUpdate::Tail {
        generation,
        page: agents::HistoryPage {
            content: vec![json!({"role":"assistant","content":[{"type":"text","text":text}]})],
            coverage: agents::HistoryCoverage::Complete,
        },
    };
    owner.apply_history_update(tail(2, "recent answer"));
    owner.apply_history_update(tail(1, "old session"));
    owner.apply_history_update(tail(2, "later revision of the same session"));
    owner.apply_command(RuntimeCommand::SelectSession {
        display_stamp: None,
        path: path.clone(),
        harness: Backend::Codex,
        session_id: path.to_string_lossy().into_owned(),
        project: project.clone(),
        key: None,
        request: Some(3),
    });
    assert_eq!(owner.history_generation, 2);
    assert!(owner.snapshot.conversation.items.is_empty());
    assert_eq!(
        owner.snapshot.history_tail.as_ref().unwrap().content.items[0].text,
        "recent answer"
    );
    assert_eq!(
        owner.snapshot.history_tail.as_ref().unwrap().coverage,
        agents::HistoryCoverage::Complete
    );
    owner.apply_history(HistoryResult {
        transcript_stamp: None,
        generation: 2,
        path,
        project,
        kind: HistoryLoadKind::Selection,
        result: Err("fixture error".into()),
    });
    owner.apply_history_update(tail(2, "late page"));
    assert!(owner.snapshot.history_tail.is_none());
    assert!(!owner.snapshot.document_ready);
    assert_eq!(owner.snapshot.status, "Could not load history");
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.is_error)
    );
}

#[test]
fn unavailable_native_history_publishes_the_readable_saved_suffix()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let project = directory.path().canonicalize()?;
    let path = project.join("missing.jsonl");
    let state = SharedStateStore::open_at(&project.join("state.sqlite3"))?;
    state.record_session_facts(
        farcaster_storage::ObservedSession {
            harness: Backend::Pi,
            project: &project,
            path: &path,
            profile_id: None,
            draft: None,
        },
        &Default::default(),
    )?;
    state.upsert_transcript_items(
        TranscriptSession {
            harness: Backend::Pi,
            project: &project,
            path: &path,
            profile_id: None,
        },
        &[(
            "saved".into(),
            json!({"role":"user","content":"saved message"}),
        )],
        false,
    )?;
    let (mut owner, events) = crate::runtime::tests::owner_without_process(project.clone());
    owner.state = Some(state);
    owner.session_id = Some("missing".into());
    let (sender, updates) = mpsc::channel();
    owner.history_tx = sender;
    owner.select_history(path.clone(), project.clone());
    owner.apply_history_update(updates.recv_timeout(Duration::from_secs(10))?);
    assert_eq!(
        owner.snapshot.saved_history,
        Some(SavedHistory::RefreshingPartial)
    );
    owner.apply_command(RuntimeCommand::SelectSession {
        display_stamp: owner.snapshot.transcript_stamp,
        path,
        harness: Backend::Pi,
        session_id: "missing".into(),
        project,
        key: None,
        request: Some(2),
    });
    let failed = updates.recv_timeout(Duration::from_secs(10))?;
    assert!(matches!(&failed, HistoryUpdate::Finished(result) if result.result.is_err()));
    owner.apply_history_update(failed);
    let published: Vec<_> = events
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. } => snapshot.saved_history,
            _ => None,
        })
        .collect();
    assert_eq!(published.last(), Some(&SavedHistory::Partial));
    assert_eq!(owner.snapshot.conversation.items[0].text, "saved message");
    Ok(())
}

#[test]
fn imported_transcript_reopens_from_sqlite_and_survives_a_failed_native_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let database = project.join("state.sqlite");
    let path = project.join("reopen.jsonl");
    let rows = [
        json!({"type":"session","version":3,"id":"reopen","cwd":project}),
        json!({"type":"message","id":"a","message":{"role":"user","content":"question"}}),
        json!({"type":"message","id":"b","parentId":"a","message":{
            "role":"assistant","content":[{"type":"text","text":"saved answer"}]}}),
    ];
    std::fs::write(
        &path,
        rows.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let open = |state: SharedStateStore| {
        let (mut owner, _) = crate::runtime::tests::owner_without_process(project.clone());
        owner.state = Some(state.clone());
        let (sender, updates) = mpsc::channel();
        owner.history_tx = sender;
        owner.select_history(path.clone(), project.clone());
        let mut used_adapter = false;
        while owner.history_selection_generation.is_some() {
            let update = updates
                .recv_timeout(Duration::from_secs(10))
                .expect("history result");
            used_adapter |= matches!(update, HistoryUpdate::Finished(_));
            owner.apply_history_update(update);
        }
        assert!(owner.snapshot.document_ready);
        assert_eq!(owner.snapshot.conversation.items.len(), 2);
        assert_eq!(owner.snapshot.conversation.items[1].text, "saved answer");
        let stamp = state
            .transcript_stamp(TranscriptSession {
                harness: Backend::Pi,
                path: &path,
                project: &project,
                profile_id: None,
            })
            .unwrap()
            .expect("import was committed");
        assert_eq!(owner.snapshot.transcript_stamp, Some(stamp));
        (owner.snapshot.saved_history.is_some(), used_adapter)
    };
    assert_eq!(
        open(SharedStateStore::open_at(&database).unwrap()),
        (false, true)
    );
    // A fresh runtime with the same source revision needs no native history read.
    assert_eq!(
        open(SharedStateStore::open_at(&database).unwrap()),
        (true, false)
    );
    std::fs::remove_file(&path).unwrap();
    // An unavailable source still leaves the saved conversation fully readable.
    assert_eq!(
        open(SharedStateStore::open_at(&database).unwrap()),
        (true, true)
    );
}

#[test]
fn refresh_preserves_linked_submission_image_in_the_published_and_reopened_transcript()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().canonicalize()?;
    let path = project.join("screenshot.jsonl");
    let database = project.join("state.sqlite3");
    let receipt = "submission-with-screenshot";
    let image = crate::protocol::PromptImage::new(ONE_PIXEL_PNG.into(), "image/png".into());
    let bytes = image.bytes()?;
    let state = SharedStateStore::open_at(&database)?;
    state.with(|store| {
        let id = store.enqueue_prompt(
            "session:screenshot",
            Backend::Pi,
            &project,
            Some(&path),
            crate::protocol::PromptMode::Normal,
            "look at this",
            std::slice::from_ref(&image),
        )?;
        store.complete_delivered_prompt(id, "session:screenshot", Some(&path), receipt, true)
    })?;

    let write_native = |include_original: bool| {
        let mut user = json!({"role":"user","content":[{"type":"text","text":"look at this"}]});
        if include_original {
            user["submissionId"] = receipt.into();
            user["content"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"image","mimeType":"image/png","data":ONE_PIXEL_PNG}));
        }
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({"type":"session","version":3,"id":"screenshot","cwd":project}),
                json!({"type":"message","id":"native-user","message":user}),
            ),
        )
    };
    let assert_visible = |snapshot: &RuntimeSnapshot| {
        assert!(snapshot.document_ready);
        assert_eq!(snapshot.conversation.items.len(), 1);
        let user = &snapshot.conversation.items[0];
        assert_eq!(user.kind, TranscriptKind::User);
        assert_eq!(user.text, "look at this");
        assert_eq!(user.images.len(), 1);
        assert_eq!(user.images[0].bytes(), bytes.as_slice());
    };
    write_native(true)?;
    let (mut owner, _) = crate::runtime::tests::owner_without_process(project.clone());
    owner.state = Some(state);
    let (sender, updates) = mpsc::channel();
    owner.history_tx = sender;
    owner.select_history(path.clone(), project.clone());
    while owner.history_selection_generation.is_some() {
        owner.apply_history_update(updates.recv_timeout(Duration::from_secs(10))?);
    }
    assert_visible(&owner.snapshot);

    write_native(false)?;
    owner.refresh_session_document(path.clone(), project.clone());
    let response = updates.recv_timeout(Duration::from_secs(10))?;
    let HistoryUpdate::Finished(result) = &response else {
        panic!("expected changed native history")
    };
    let refreshed = result.result.as_ref().expect("import");
    assert_eq!(refreshed.messages.len(), 1);
    assert_eq!(refreshed.messages[0]["nativeMessageId"], "native-user");
    assert_eq!(refreshed.messages[0]["submissionId"], receipt);
    owner.apply_history_update(response);
    assert_visible(&owner.snapshot);
    assert!(owner.snapshot.saved_history.is_none());
    drop(owner);

    std::fs::remove_file(&path)?;
    let (mut owner, _) = crate::runtime::tests::owner_without_process(project.clone());
    owner.state = Some(SharedStateStore::open_at(&database)?);
    let (sender, updates) = mpsc::channel();
    owner.history_tx = sender;
    owner.select_history(path, project);
    let saved = updates.recv_timeout(Duration::from_secs(10))?;
    assert!(matches!(saved, HistoryUpdate::Saved { .. }));
    owner.apply_history_update(saved);
    assert_visible(&owner.snapshot);
    assert!(owner.snapshot.saved_history.is_some());
    while owner.history_selection_generation.is_some() {
        owner.apply_history_update(updates.recv_timeout(Duration::from_secs(10))?);
    }
    assert_visible(&owner.snapshot);
    Ok(())
}
