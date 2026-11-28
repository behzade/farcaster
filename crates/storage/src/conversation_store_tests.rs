use super::*;
use base64::Engine as _;

fn session<'a>(project: &'a Path, path: &'a Path) -> TranscriptSession<'a> {
    TranscriptSession {
        harness: Backend::Codex,
        path,
        project,
        profile_id: None,
    }
}

#[test]
fn persisted_transcript_reopens_by_scope_and_tail_without_the_ui_store_lock() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("app.sqlite");
    let project = dir.path();
    let locator = project.join("test");
    let s = session(project, &locator);
    let repo = SharedStateStore::open_at(&db)?;
    let image = base64::engine::general_purpose::STANDARD.encode(b"sample image");
    let msgs = vec![
        json!({"role":"user","submissionId":"s","content":[{"type":"text","text":"inspect"},{"type":"image","mimeType":"image/png","data":image}]}),
        json!({"role":"assistant","nativeMessageId":"msg","content":[{"type":"toolCall","id":"call","arguments":{"text":"abc"}}]}),
        json!({"role":"toolResult","toolCallId":"call","content":[{"type":"text","text":"abc"}],"adapter":{"exitCode":0}}),
    ];
    let metadata = json!({"model":"model","version":"revision"});
    let ui = repo.lock()?;
    assert!(repo.replace_transcript(s, 0, &msgs, &metadata)?);
    let body: String = ui.connection.query_row(
        "SELECT body FROM transcript_messages WHERE ordinal=0",
        [],
        |r| r.get(0),
    )?;
    assert!(!body.contains(&image));
    assert_eq!(std::fs::read_dir(project.join("images"))?.count(), 1);
    drop(ui);
    let reopened = SharedStateStore::open_at(&db)?;
    let page = reopened.load_transcript(s, Some(2))?.unwrap();
    assert_eq!(page.messages, &msgs[1..]);
    assert!(!page.complete);
    assert!(page.complete_from_start);
    assert_eq!(page.total, 3);
    let all = reopened.load_transcript(s, None)?.unwrap();
    assert!(all.complete);
    assert_eq!(all.messages, msgs);
    assert_eq!(all.metadata, metadata);
    let sibling = TranscriptSession {
        harness: Backend::Pi,
        ..s
    };
    let other_profile = TranscriptSession {
        profile_id: Some("sibling"),
        ..s
    };
    assert!(reopened.load_transcript(sibling, None)?.is_none());
    assert!(reopened.load_transcript(other_profile, None)?.is_none());
    assert!(
        reopened
            .replace_transcript(other_profile, 0, &[], &Value::Null)
            .is_err()
    );
    assert!(reopened.replace_transcript(sibling, 0, &[], &Value::Null)?);
    Ok(())
}

#[test]
fn incremental_transcript_writes_reuse_the_same_message_and_reject_stale_imports() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = SharedStateStore::open_at(&dir.path().join("state.sqlite"))?;
    let path = dir.path().join("transcript");
    let session = session(dir.path(), &path);
    let first = json!({"role":"assistant","nativeMessageId":"one","content":"first"});
    assert!(repo.replace_transcript(session, 0, std::slice::from_ref(&first), &Value::Null)?);
    let revision = repo.load_transcript(session, Some(0))?.unwrap().revision;
    let second = json!({"role":"toolResult","toolCallId":"cmd","content":"running"});
    repo.upsert_transcript_items(session, &[("tool".into(), second.clone())], false)?;
    let mut done = second;
    done["content"] = "done".into();
    repo.upsert_transcript_items(session, &[("tool".into(), done.clone())], false)?;
    assert!(!repo.replace_transcript(session, revision, &[], &Value::Null)?);
    let read = repo.load_transcript(session, None)?.unwrap();
    assert!(read.complete);
    assert_eq!(read.messages, [first.clone(), done.clone()]);
    assert!(repo.replace_transcript(
        session,
        read.revision,
        &[first, done.clone()],
        &Value::Null
    )?);
    repo.upsert_transcript_items(session, &[("different actor".into(), done)], false)?;
    assert_eq!(repo.load_transcript(session, None)?.unwrap().total, 2);
    Ok(())
}

#[test]
fn durable_draft_and_native_session_merge_retains_transcript_and_submission_images() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = SharedStateStore::open_at(&dir.path().join("state.sqlite"))?;
    let path = dir.path().join("p");
    let session = session(dir.path(), &path);
    let seen =
        json!({"role":"user","nativeMessageId":"u","submissionId":"receipt","content":"run"});
    assert!(store.replace_transcript(session, 0, &[seen], &Value::Null)?);
    let mut db = store.lock()?;
    let original = session.find(&db.connection)?.unwrap();
    let image = PromptImage::new("YWJj".into(), "image/png".into());
    let (images, _) =
        db.encode_prompt_images(&[image, PromptImage::new("ZGVm".into(), "image/png".into())])?;
    let tx = db.connection.transaction()?;
    tx.execute("INSERT INTO sessions(project_id,harness,created_ms,modified_ms,client_key) SELECT project_id,harness,0,0,'draft:owned' FROM sessions WHERE id=?1",[original])?;
    let keep = tx.last_insert_rowid();
    tx.execute("INSERT INTO outbox(session_id,mode,message,display_message,invocation,images_json,created_ms) VALUES(?1,'normal','run','/go','expanded',?2,0)",params![keep,images])?;
    let outbox = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO session_events(session_id,seq,t,schema_version,body) VALUES(?1,1,0,1,?2)",
        params![
            keep,
            json!({"type":"accepted_prompt","submissionId":"receipt","message":"run","outboxId":outbox,"images":serde_json::from_str::<Value>(&images)?})
                .to_string()
        ],
    )?;
    super::super::identity::bind_locator(
        &tx,
        "draft:owned",
        &sessions::normalize_session_path(&path),
    )?;
    tx.commit()?;
    drop(db);
    let read = store.load_transcript(session, None)?.unwrap();
    let msg = json!({"role":"user","nativeMessageId":"u","content":[{"type":"text","text":"run"},{"type":"image","mimeType":"image/png","data":"eA=="}]});
    assert!(store.replace_transcript(
        session,
        read.revision,
        std::slice::from_ref(&msg),
        &Value::Null
    )?);
    let mut adapter_result = Arc::new(vec![msg]);
    store.restore_transcript_links(session, &mut adapter_result)?;
    assert_eq!(adapter_result[0]["submissionId"], "receipt");
    assert_eq!(adapter_result[0]["content"][1]["data"], "YWJj");
    assert_eq!(adapter_result[0]["content"][2]["data"], "ZGVm");
    let mut newer = Arc::new(vec![json!({"role":"user","content":"newer import"})]);
    let untouched = Arc::clone(&newer);
    store.restore_transcript_links(session, &mut newer)?;
    assert!(Arc::ptr_eq(&untouched, &newer));
    let read = store.load_transcript(session, None)?.unwrap();
    assert_eq!(read.total, 1);
    assert_eq!(read.messages[0]["content"][1]["data"], "YWJj");
    assert_eq!(read.messages[0]["farcasterUserInvocation"], "/go");
    assert_eq!(read.messages[0]["submissionId"], "receipt");
    assert_eq!(read.messages[0]["content"][2]["data"], "ZGVm");
    let display = store.load_transcript_display(session, false)?.unwrap();
    assert!(!display.full_document);
    let prompt = &display.content.items[0];
    assert_eq!(prompt.images.len(), 2);
    assert_eq!(prompt.images[0].bytes(), b"abc");
    assert_eq!(prompt.images[1].bytes(), b"def");
    store.with(|db| {
        db.connection
            .execute("DELETE FROM outbox", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    let without_outbox = store.load_transcript(session, None)?.unwrap();
    assert_eq!(without_outbox.messages[0]["content"][2]["data"], "ZGVm");
    Ok(())
}

#[test]
fn import_preserves_exact_tool_arguments_and_does_not_link_ambiguous_native_ids() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("app.sqlite");
    let repo = SharedStateStore::open_at(&db)?;
    let path = dir.path().join("s");
    let s = session(dir.path(), &path);
    repo.replace_transcript(s, 0, &[], &Value::Null)?;
    let call = json!({"id":"tool","name":"command","arguments":{"cmd":"pwd"}});
    let tool = json!({"role":"toolResult","toolCallId":"tool","content":"finished"});
    let user = json!({"role":"user","content":"same","nativeMessageId":"old-id"});
    let mut live_tool = tool.clone();
    live_tool["farcasterToolCall"] = call.clone();
    live_tool["details"] = json!({"native":{"result":{"structuredContent":{"id":123}}}});
    let mut sent = user.clone();
    sent["submissionId"] = "receipt".into();
    repo.upsert_transcript_items(
        s,
        &[("user".into(), sent), ("result".into(), live_tool)],
        true,
    )?;
    let revision = repo.load_transcript(s, Some(0))?.unwrap().revision;
    assert!(repo.replace_transcript(s, revision, &[user.clone(), user, tool], &Value::Null)?);
    let restored = repo.load_transcript(s, None)?.unwrap().messages;
    assert_eq!(restored[2]["farcasterToolCall"], call);
    let mut imported = restored.clone();
    imported[2]
        .as_object_mut()
        .unwrap()
        .remove("farcasterToolCall");
    imported[2].as_object_mut().unwrap().remove("details");
    let mut imported = Arc::new(imported);
    repo.restore_transcript_links(s, &mut imported)?;
    assert_eq!(imported[2]["farcasterToolCall"], call);
    assert_eq!(
        imported[2]["details"]["native"]["result"]["structuredContent"]["id"],
        123
    );
    assert!(restored[0].get("submissionId").is_none());
    assert!(restored[1].get("submissionId").is_none());
    Ok(())
}

#[test]
fn unchanged_imports_and_appends_leave_the_old_prefix_on_disk() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = SharedStateStore::open_at(&dir.path().join("state.sqlite"))?;
    let path = dir.path().join("s");
    let s = session(dir.path(), &path);
    let mut messages = vec![json!({"role":"assistant","content":"anonymous prefix"})];
    let meta = json!({"model":"model"});
    assert!(repo.replace_transcript(s, 0, &messages, &meta)?);
    repo.with(|store| store.connection.execute_batch(
        "CREATE TRIGGER preserve_prefix BEFORE UPDATE ON transcript_messages WHEN OLD.ordinal=0 BEGIN SELECT RAISE(ABORT,'prefix rewritten'); END;
         CREATE TRIGGER retain_prefix BEFORE DELETE ON transcript_messages WHEN OLD.ordinal=0 BEGIN SELECT RAISE(ABORT,'prefix removed'); END;"
    ).map_err(|e|e.to_string()))?;
    assert!(repo.replace_transcript(s, 1, &messages, &meta)?);
    assert_eq!(repo.load_transcript(s, Some(0))?.unwrap().revision, 1);
    messages.push(json!({"role":"user","content":"new question"}));
    assert!(repo.replace_transcript(s, 1, &messages, &meta)?);
    assert_eq!(repo.load_transcript(s, None)?.unwrap().messages, messages);
    Ok(())
}

#[test]
fn viewport_tail_seeks_a_user_turn_before_a_long_run_of_tool_calls() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = SharedStateStore::open_at(&dir.path().join("state.sqlite"))?;
    let path = dir.path().join("history");
    let s = session(dir.path(), &path);
    let messages:Vec<_>=(0..200).map(|i|json!({"role":if i==85||i==177||i==198 {"user"} else {"toolResult"},"content":format!("item {i}")})).collect();
    repo.replace_transcript(s, 0, &messages, &Value::Null)?;
    let ordinary = repo.load_transcript(s, Some(64))?.unwrap();
    assert_eq!(ordinary.messages.len(), 64);
    let tail = repo.load_transcript_tail(s, 64)?.unwrap();
    assert_eq!(tail.messages, messages[85..]);
    assert!(!tail.complete);
    Ok(())
}

#[test]
fn v24_database_keeps_preexisting_prompt_files_when_history_is_imported() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("state.sqlite");
    let path = dir.path().join("transcript");
    let s = session(dir.path(), &path);
    {
        let mut old = StateStore::open_at(&database)?;
        old.connection.execute_batch(
            "DROP TABLE transcript_messages;
             DROP TABLE transcript_documents;
             DROP INDEX transcript_accepted_prompt;
             UPDATE meta SET value='24' WHERE key='schema_version';",
        )?;
        let (images, _) =
            old.encode_prompt_images(&[PromptImage::new("YWJj".into(), "image/png".into())])?;
        let tx = old.connection.transaction()?;
        let id = s.ensure(&tx)?;
        tx.execute(
            "INSERT INTO session_events(session_id,seq,t,schema_version,body) VALUES(?1,1,0,1,?2)",
            params![id,json!({"type":"accepted_prompt","submissionId":"legacy","images":serde_json::from_str::<Value>(&images)?}).to_string()]
        )?;
        tx.commit()?;
    }
    let repo = SharedStateStore::open_at(&database)?;
    assert!(repo.load_transcript(s, None)?.is_none());
    let incoming = json!({"role":"user","content":"one attachment","submissionId":"legacy"});
    assert!(repo.replace_transcript(s, 0, &[incoming], &Value::Null)?);
    let stored = repo.load_transcript(s, None)?.unwrap();
    assert_eq!(stored.messages[0]["content"][1]["data"], "YWJj");
    assert_eq!(std::fs::read_dir(dir.path().join("images"))?.count(), 1);
    Ok(())
}

#[test]
fn failed_replacement_keeps_the_old_rows_and_revision() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let repo = SharedStateStore::open_at(&dir.path().join("state.sqlite"))?;
    let path = dir.path().join("s");
    let s = session(dir.path(), &path);
    let first = json!({"role":"assistant","content":"original"});
    assert!(repo.replace_transcript(s, 0, std::slice::from_ref(&first), &Value::Null)?);
    repo.with(|store| {
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_item BEFORE INSERT ON transcript_messages WHEN NEW.ordinal=1
         BEGIN SELECT RAISE(ABORT,'simulated write failure'); END;",
            )
            .map_err(|e| e.to_string())
    })?;
    let changed = json!({"role":"assistant","content":"replacement"});
    assert!(
        repo.replace_transcript(s, 1, &[changed.clone(), changed], &Value::Null)
            .is_err()
    );
    let recovered = repo.load_transcript(s, None)?.unwrap();
    assert_eq!(recovered.revision, 1);
    assert_eq!(recovered.messages, [first]);
    Ok(())
}

#[test]
fn transcript_stamp_tracks_same_length_edits_and_prompt_receipts_from_another_connection()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("state.sqlite");
    let path = dir.path().join("thread");
    let session = session(dir.path(), &path);
    let reader = SharedStateStore::open_at(&db)?;
    let writer = SharedStateStore::open_at(&db)?;
    let first = json!({"role":"assistant","content":"old"});
    let second = json!({"role":"assistant","content":"new"});
    assert!(reader.transcript_stamp(session)?.is_none());
    let original = writer
        .replace_transcript_with_stamp(session, 0, &[first], &Value::Null)?
        .unwrap();
    assert_eq!(
        reader.load_transcript(session, None)?.unwrap().stamp,
        original
    );
    let edited = writer
        .replace_transcript_with_stamp(session, original.revision, &[second], &Value::Null)?
        .unwrap();
    assert_ne!(original.revision, edited.revision);
    assert_eq!(reader.transcript_stamp(session)?, Some(edited));
    let raw = writer.lock()?;
    let append = |event_type| {
        raw.connection.execute(
            "INSERT INTO session_events(session_id,seq,t,schema_version,body) SELECT ?1,coalesce(max(seq),0)+1,0,1,json_object('type',?2) FROM session_events WHERE session_id=?1",
            params![edited.session_id,event_type],
        )
    };
    append("archive_session")?;
    assert_eq!(reader.transcript_stamp(session)?, Some(edited));
    append("prompt_delivery_receipt")?;
    let receipt = reader.transcript_stamp(session)?.unwrap();
    assert_eq!(receipt.revision, edited.revision);
    assert!(receipt.prompt_event_seq > edited.prompt_event_seq);
    assert_eq!(
        reader.transcript_prompt_overlay(session, None)?.stamp,
        Some(receipt)
    );
    assert_eq!(
        reader.load_transcript(session, None)?.unwrap().stamp,
        receipt
    );
    Ok(())
}
