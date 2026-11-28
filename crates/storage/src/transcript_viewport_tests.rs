use super::*;
use farcaster_conversation::viewport_item_matches;
use serde_json::{Value, json};

fn messages_with_expanded_prefix(first: Value) -> Vec<Value> {
    let mut messages = vec![first];
    for index in 1..80 {
        let role = if index == 20 { "user" } else { "assistant" };
        messages.push(
            json!({"role":role, "content":[{"type":"text", "text":format!("line {index}")}]}),
        );
    }
    messages
}

fn assert_original_tail(page: StoredViewport) {
    assert!(!page.full_document);
    assert_eq!(page.coverage, agents::HistoryCoverage::Suffix);
    assert_eq!(page.content.items.len(), TAIL_MESSAGES);
    assert_eq!(page.content.items[0].text, "line 16");
    assert_eq!(page.content.items.last().unwrap().text, "line 79");
}

#[test]
fn oversized_earlier_turn_keeps_the_compact_tail() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let repo = SharedStateStore::open_at(&tmp.path().join("state.sqlite"))?;
    let path = tmp.path().join("thread");
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let messages = messages_with_expanded_prefix(
        json!({"role":"user", "content":"x".repeat(MAX_VIEWPORT_BYTES + 1)}),
    );
    repo.replace_transcript(session, 0, &messages, &json!({}))?;
    assert_original_tail(repo.load_transcript_display(session, false)?.unwrap());
    Ok(())
}

#[test]
fn changed_prefix_images_recover_the_original_tail_before_and_after_preparation()
-> Result<(), Error> {
    let tmp = tempfile::tempdir()?;
    let store = StateStore::open_at(&tmp.path().join("state.sqlite"))?;
    let path = tmp.path().join("thread");
    let session = TranscriptSession {
        harness: Backend::Codex,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let image = json!({"type":"image","data":"AQID","mimeType":"image/png"});
    let messages = messages_with_expanded_prefix(json!({"role":"user","content":[image,image]}));
    assert!(
        store
            .replace_conversation(session, 0, &messages, &json!({}))?
            .is_some()
    );
    let cached = store.read_display(session, false)?.unwrap();
    assert!(!cached.full_document);
    assert_eq!(cached.coverage, agents::HistoryCoverage::Complete);
    let id = session.find(&store.connection)?.unwrap();
    let attachment = std::fs::read_dir(&store.image_directory)?
        .next()
        .unwrap()?
        .path();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&attachment)?
        .set_len(MAX_INPUT_BYTES as u64 / 8 * 3)?;
    assert_original_tail(store.read_display(session, false)?.unwrap());
    let tx = store.connection.unchecked_transaction()?;
    assert!(store.viewport_messages(&tx, id, 0)?.is_none());
    tx.commit()?;

    std::fs::remove_file(attachment)?;
    assert_original_tail(store.read_display(session, false)?.unwrap());
    store
        .connection
        .execute("DELETE FROM transcript_viewports WHERE session_id=?1", [id])?;
    store.prepare_viewport(id)?;
    assert_original_tail(store.read_display(session, false)?.unwrap());
    assert!(store.read_display(session, true).is_err());
    Ok(())
}

#[test]
fn inline_receipt_is_checked_against_remaining_body_budget_before_parsing() -> Result<(), Error> {
    let tmp = tempfile::tempdir()?;
    let store = StateStore::open_at(&tmp.path().join("state.sqlite"))?;
    let path = tmp.path().join("thread");
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let messages = messages_with_expanded_prefix(
        json!({"role":"user","submissionId":"oversize","content":"x".repeat(7 * 1024 * 1024)}),
    );
    assert!(
        store
            .replace_conversation(session, 0, &messages, &json!({}))?
            .is_some()
    );
    let id = session.find(&store.connection)?.unwrap();
    let receipt = json!({"type":"accepted_prompt","submissionId":"oversize","images":["x".repeat(2 * 1024 * 1024)]});
    store.connection.execute(
        "INSERT INTO session_events(session_id,seq,t,schema_version,body) VALUES(?1,1,0,1,?2)",
        params![id, receipt.to_string()],
    )?;
    let tx = store.connection.unchecked_transaction()?;
    assert!(store.viewport_messages(&tx, id, 0)?.is_none());
    tx.commit()?;
    store.prepare_viewport(id)?;
    assert_original_tail(store.read_display(session, false)?.unwrap());
    Ok(())
}

#[test]
fn oversized_cached_json_is_rejected_before_deserialization() -> Result<(), Error> {
    let tmp = tempfile::tempdir()?;
    let store = StateStore::open_at(&tmp.path().join("state.sqlite"))?;
    let error = store
        .decode_viewport(&vec![b'x'; MAX_VIEWPORT_BYTES + 1])
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::FileTooLarge
    );
    Ok(())
}

#[test]
fn transcript_reads_reuse_an_idle_connection_and_allow_an_overlapping_read()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let repo = SharedStateStore::open_at(&tmp.path().join("state.sqlite"))?;
    repo.with_transcript_connection(|store| {
        store
            .connection
            .execute_batch("CREATE TEMP TABLE reused(value)")
            .map_err(|e| e.to_string())
    })?;
    repo.with_transcript_connection(|store| {
        let exists = |store: &StateStore| -> Result<bool, String> {
            store
                .connection
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM sqlite_temp_master WHERE name='reused')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())
        };
        assert!(exists(store)?);
        assert!(repo.1.as_ref().unwrap().idle_connection.try_lock().is_ok());
        repo.with_transcript_connection(|other| {
            assert!(!exists(other)?);
            Ok(())
        })
    })?;
    Ok(())
}

#[test]
fn prepared_group_keeps_header_and_source_identity_and_omits_expanded_output()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let db = tmp.path().join("state.sqlite");
    let repo = SharedStateStore::open_at(&db)?;
    let path = tmp.path().join("thread");
    let s = TranscriptSession {
        harness: Backend::Codex,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let output = "lots of hidden output".repeat(2500);
    let mut msgs = vec![json!({"role":"user","content":"please run checks"})];
    for id in ["one", "two"] {
        msgs.push(json!({"role":"assistant","content":[{"type":"toolCall","name":"bash","id":id,"arguments":{"command":"pwd"},"toolMetadata":{"category":"execute"}}]}));
        msgs.push(
            json!({"role":"toolResult","toolCallId":id,"content":[{"type":"text","text":output}]}),
        );
    }
    msgs.push(json!({"role":"assistant","content":[{"type":"text","text":"all done"}]}));
    assert!(repo.replace_transcript(s, 0, &msgs, &json!({}))?);
    let original = repo.load_transcript(s, None)?.unwrap();
    let mut full = ConversationState::default();
    full.replace_history(&original.messages);
    let guard = repo.lock()?;
    let size: usize = guard.connection.query_row(
        "SELECT octet_length(items) FROM transcript_viewports",
        [],
        |r| r.get(0),
    )?;
    assert!(size < 10_000);
    let viewport = repo.load_transcript_display(s, false)?.unwrap();
    assert_eq!(viewport.coverage, agents::HistoryCoverage::Complete);
    assert_eq!(viewport.content.items.len(), full.items.len());
    for (preview, source) in viewport.content.items.iter().zip(full.items.iter()) {
        assert!(viewport_item_matches(preview, source));
        assert_eq!(
            preview.tool_execution_state(),
            source.tool_execution_state()
        );
    }
    assert!(!viewport_item_matches(
        &viewport.content.items[1],
        &full.items[2]
    ));
    drop(guard);
    Ok(())
}

#[test]
fn saved_image_rows_share_attachment_files_and_restore_their_original_items()
-> Result<(), Box<dyn std::error::Error>> {
    use base64::Engine as _;
    let tmp = tempfile::tempdir()?;
    let db = tmp.path().join("state.sqlite");
    let path = tmp.path().join("session");
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let repo = SharedStateStore::open_at(&db)?;
    let png = include_bytes!("../../../assets/icons/app/icon_128x128.png");
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    let image = json!({"type":"image","data":data,"mimeType":"image/png"});
    let messages = [
        json!({"role":"user","content":[{"type":"text","text":"first"},image]}),
        json!({"role":"assistant","content":[{"type":"text","text":"saw it"}]}),
        json!({"role":"user","content":[{"type":"text","text":"same"},image]}),
    ];
    repo.replace_transcript(session, 0, &messages, &json!({}))?;
    let store = repo.lock()?;
    let bytes: Vec<u8> =
        store
            .connection
            .query_row("SELECT items FROM transcript_viewports", [], |r| r.get(0))?;
    assert!(bytes.len() < 3000);
    let images = std::fs::read_dir(&store.image_directory)?.count();
    assert_eq!(images, 1);
    drop(store);
    drop(repo);
    let repo = SharedStateStore::open_at(&db)?;
    let compact = repo.load_transcript_display(session, false)?.unwrap();
    let full = repo.load_transcript_display(session, true)?.unwrap();
    assert!(!compact.full_document);
    assert!(full.full_document);
    assert_eq!(compact.content.items, full.content.items);
    let first = &compact.content.items[0].images[0];
    let last = &compact.content.items[2].images[0];
    assert!(Arc::ptr_eq(first, last));
    assert_eq!(first.bytes(), png);
    Ok(())
}

#[test]
fn command_presentation_in_saved_tail_matches_full_history_after_upgrade() -> Result<(), Error> {
    let tmp = tempfile::tempdir()?;
    let db = tmp.path().join("state.sqlite");
    let path = tmp.path().join("thread");
    let session = TranscriptSession {
        harness: Backend::Codex,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let store = StateStore::open_at(&db)?;
    let mut messages: Vec<_> = (0..100)
        .map(|i| json!({"role":"assistant","content":[{"type":"text","text":format!("message {i}")}]}))
        .collect();
    for (ordinal, id) in [(0, "older"), (34, "boundary"), (95, "recent")] {
        messages[ordinal] = json!({"role":"user","submissionId":id,"content":"expanded"});
    }
    messages[98] = json!({"role":"user","content":"expanded"});
    store.replace_conversation(session, 0, &messages, &json!({}))?;
    let id = session.find(&store.connection)?.unwrap();
    for (seq, submission, title) in [(1, "older", "$old"), (2, "recent", "$new")] {
        let event = json!({"type":"prompt_presentation","submissionId":submission,
                           "resolved":"expanded","display":title,"invocation":"command"});
        store.connection.execute(
            "INSERT INTO session_events VALUES(?1,?2,0,1,?3)",
            params![id, seq, event.to_string()],
        )?;
    }
    let check = |store: &StateStore| -> Result<(), Error> {
        let page = store.read_display(session, false)?.unwrap();
        let full = store.read_display(session, true)?.unwrap();
        assert!(!page.full_document);
        assert_eq!(page.coverage, agents::HistoryCoverage::Suffix);
        let offset = full.content.items.len() - page.content.items.len();
        assert!(offset > 0 && offset < 95);
        for (i, item) in page.content.items.iter().enumerate() {
            assert!(viewport_item_matches(item, &full.content.items[offset + i]));
        }
        assert_eq!(page.content.items[95 - offset].text, "$new");
        assert_eq!(page.content.items[98 - offset].text, "expanded");
        Ok(())
    };
    store.connection.execute_batch(
        "UPDATE transcript_viewports SET items=NULL,event_seq=2;
                                    UPDATE meta SET value='28' WHERE key='schema_version'",
    )?;
    drop(store);
    let store = StateStore::open_at(&db)?;
    check(&store)?;
    store.prepare_viewport(id)?;
    check(&store)?;
    assert!(!store.can_display_history_tail(&path)?);
    store.connection.execute("INSERT INTO session_events VALUES(?1,3,0,1,?2)",
        params![id,json!({"type":"prompt_presentation","resolved":"expanded","display":"legacy","invocation":"command"}).to_string()])?;
    assert!(store.read_display(session, false)?.unwrap().full_document);
    Ok(())
}

#[test]
fn v26_upgrade_rebuilds_the_short_page_without_changing_the_document()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let db = tmp.path().join("state.sqlite");
    let path = tmp.path().join("thread");
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let repo = SharedStateStore::open_at(&db)?;
    let messages: Vec<_> = (0..100)
        .map(|i| {
            json!({
                "role": if i == 0 || i == 82 { "user" } else { "assistant" },
                "content": [{"type":"text","text":format!("message {i}")}],
            })
        })
        .collect();
    repo.replace_transcript(session, 0, &messages, &json!({}))?;
    let original = repo.transcript_stamp(session)?;
    repo.with(|db| db.connection.execute_batch(
        "UPDATE transcript_viewports SET from_beginning=0; UPDATE meta SET value='26' WHERE key='schema_version';"
    ).map_err(|e|e.to_string()))?;
    drop(repo);
    let repo = SharedStateStore::open_at(&db)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let page = repo.load_transcript_display(session, false)?.unwrap();
        if !page.full_document && page.coverage == agents::HistoryCoverage::Complete {
            assert_eq!(page.content.items.len(), messages.len());
            break;
        }
        assert!(Instant::now() < deadline, "old viewport was not replaced");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(repo.transcript_stamp(session)?, original);
    Ok(())
}

#[test]
fn migration_backfills_existing_transcript_and_late_receipt_invalidates_its_viewport()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let db = tmp.path().join("state.sqlite");
    let path = tmp.path().join("session");
    let session = TranscriptSession {
        harness: Backend::Pi,
        path: &path,
        project: tmp.path(),
        profile_id: None,
    };
    let repo = SharedStateStore::open_at(&db)?;
    repo.replace_transcript(
        session,
        0,
        &[json!({"role":"user","content":"saved before upgrade"})],
        &json!({}),
    )?;
    repo.with(|store|store.connection.execute_batch("DROP TABLE transcript_viewports; UPDATE meta SET value='25' WHERE key='schema_version';").map_err(|e|e.to_string()))?;
    drop(repo);
    let reopened = SharedStateStore::open_at(&db)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while reopened
        .load_transcript_display(session, false)?
        .is_none_or(|page| page.full_document)
    {
        assert!(
            Instant::now() < deadline,
            "backfill did not produce a viewport"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let store = reopened.lock()?;
    let id = session.find(&store.connection)?.unwrap();
    store.connection.execute(
        "INSERT INTO session_events VALUES(?1,1,0,1,?2)",
        params![id, json!({"type":"session_metrics"}).to_string()],
    )?;
    drop(store);
    assert!(
        !reopened
            .load_transcript_display(session, false)?
            .unwrap()
            .full_document
    );
    let store = reopened.lock()?;
    store.connection.execute(
        "INSERT INTO session_events VALUES(?1,2,0,1,?2)",
        params![
            id,
            json!({"type":"accepted_prompt","submissionId":"new","deliveryTracked":true,"message":"sent locally","images":[]})
                .to_string()
        ],
    )?;
    drop(store);
    let display = reopened.load_transcript_display(session, false)?.unwrap();
    assert!(display.full_document);
    assert_eq!(display.content.items.last().unwrap().text, "sent locally");
    Ok(())
}

#[test]
#[ignore = "measures existing isolated transcripts from FARCASTER_TRANSCRIPT_BENCH_DB"]
fn measure_stored_viewport_decode_on_isolated_transcripts() -> Result<(), Box<dyn std::error::Error>>
{
    let path = PathBuf::from(std::env::var("FARCASTER_TRANSCRIPT_BENCH_DB")?);
    let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let store = StateStore {
        connection: db,
        image_directory: path.parent().unwrap().join("images"),
    };
    let addresses=store.connection.prepare(
        "SELECT s.harness,s.locator,p.path,s.profile_id,d.item_count FROM transcript_documents d
         JOIN sessions s ON s.id=d.session_id JOIN projects p ON p.id=s.project_id ORDER BY d.item_count"
    )?.query_map([],|r|Ok((r.get::<_,String>(0)?,PathBuf::from(r.get::<_,String>(1)?),PathBuf::from(r.get::<_,String>(2)?),r.get::<_,Option<String>>(3)?,r.get::<_,usize>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (backend, path, project, profile, total) in addresses {
        let session = TranscriptSession {
            harness: backend.parse()?,
            path: &path,
            project: &project,
            profile_id: profile.as_deref(),
        };
        let page = store
            .read_conversation(session, Some(TAIL_MESSAGES), true)?
            .unwrap();
        let Some(bytes) = store.encode_page(&page.messages, &project)? else {
            continue;
        };
        let mut normal = Vec::new();
        let mut decoded = Vec::new();
        for _ in 0..9 {
            let before = Instant::now();
            let page = store
                .read_conversation(session, Some(TAIL_MESSAGES), true)?
                .unwrap();
            let mut full = ConversationState::default();
            full.replace_history(&page.messages);
            std::hint::black_box(&full.items);
            normal.push(before.elapsed());
            let before = Instant::now();
            let (prepared, _) = store.decode_viewport(&bytes)?;
            std::hint::black_box(&prepared.items);
            decoded.push(before.elapsed());
        }
        normal.sort();
        decoded.sort();
        use std::io::Write as _;
        writeln!(
            std::io::stdout().lock(),
            "viewport bench messages={total} page={} cache_bytes={} query_project={:?} prepared_decode={:?}",
            page.messages.len(),
            bytes.len(),
            normal[4],
            decoded[4]
        )?;
    }
    Ok(())
}
