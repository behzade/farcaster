use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::SystemTime,
};

use rusqlite::{Connection, params};
use tempfile::tempdir;

use crate::{
    projects::{DraftSession, Registry},
    protocol::{PromptImage, PromptMode},
    sessions::{SessionSummary, UsageSummary},
    state::{ComposerRecord, StateStore, WindowPlacement, WindowState},
};

#[test]
fn window_placement_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let placement = WindowPlacement {
        bounds: [-1800.0, 40.0, 1240.0, 820.0],
        display_uuid: Some("external-display".into()),
        display_origin: [-1920.0, 0.0],
        state: WindowState::Maximized,
    };

    StateStore::open_at(&database)?.save_window_placement(&placement)?;

    assert_eq!(
        StateStore::open_at(&database)?.load_window_placement()?,
        Some(placement)
    );
    Ok(())
}

#[test]
fn registry_composer_and_outbox_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let database = temp.path().join("state/gui.sqlite3");
    let session_path = temp.path().join("submitted-session.jsonl");
    fs::write(&session_path, "{}")?;
    let draft = DraftSession {
        id: "draft-one".into(),
        app_session_id: 1,
        project: project.clone(),
        created_ms: 7,
        submitted: true,
        session_path: Some(session_path.clone()),
        title: Some("Provisional title".into()),
    };
    {
        let mut store = StateStore::open_at(&database)?;
        store.save_registry(&Registry {
            projects: vec![project.clone()],
            drafts: vec![draft.clone()],
        })?;
        store.save_app_session_order(&[7, 3, 1])?;
        store.enqueue_prompt(
            "draft:draft-one",
            &project,
            None,
            PromptMode::Normal,
            "hello",
            &[PromptImage::new("aGVsbG8=".into(), "image/png".into())],
        )?;
        store.save_composer_session(&ComposerRecord {
            target: "draft:draft-one".into(),
            text: "draft text".into(),
            cursor: 6,
            selection_start: 2,
            selection_end: 6,
            history: vec!["new".into(), "old".into()],
        })?;
        let catalog_session_path = temp.path().join("session.jsonl");
        fs::write(&catalog_session_path, "{}")?;
        store.replace_sessions(&[SessionSummary::from_cached(
            "session-one".into(),
            catalog_session_path.canonicalize()?,
            project.canonicalize()?,
            "literal_100%".into(),
            "hello".into(),
            "2026-08-15T00:00:00Z".into(),
            None,
            SystemTime::now(),
            1,
            UsageSummary::default(),
            false,
            true,
            "literal_100% hello".into(),
        )])?;
        store.set_settled(&catalog_session_path.canonicalize()?, true)?;
    }
    let mut store = StateStore::open_at(&database)?;
    assert_eq!(
        store.load_registry()?.drafts,
        vec![DraftSession {
            project: project.canonicalize()?,
            session_path: Some(session_path.canonicalize()?),
            ..draft
        }]
    );
    assert_eq!(store.load_app_session_order()?, vec![7, 3, 1]);
    let queued = store.queued_prompts()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message, "hello");
    assert_eq!(
        queued[0].images,
        vec![PromptImage::new("aGVsbG8=".into(), "image/png".into())]
    );
    assert_eq!(
        store.load_composer_sessions()?,
        vec![ComposerRecord {
            target: "draft:draft-one".into(),
            text: "draft text".into(),
            cursor: 6,
            selection_start: 2,
            selection_end: 6,
            history: vec!["new".into(), "old".into()],
        }]
    );
    assert_eq!(store.cached_sessions("literal_100%")?.len(), 1);
    assert!(store.cached_sessions("")?[0].settled);
    assert!(store.cached_sessions("")?[0].is_running);
    store.begin_prompt(queued[0].id)?;
    store.complete_prompt(
        queued[0].id,
        "draft:draft-one",
        Some(&session_path.canonicalize()?),
    )?;
    assert!(store.queued_prompts()?.is_empty());
    store.delete_composer_session("draft:draft-one")?;
    assert!(store.load_composer_sessions()?.is_empty());
    Ok(())
}

#[test]
fn application_session_ids_are_incremental_i64_values() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    let first = store.allocate_app_session_id("first", 1)?;
    let second = store.allocate_app_session_id("second", 2)?;

    assert!(first > 0);
    assert_eq!(second, first + 1);
    Ok(())
}

#[test]
fn prompt_completion_persists_draft_session_association_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let session = temp.path().join("session.jsonl");
    fs::write(&session, "{}")?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    store.save_registry(&Registry {
        projects: vec![project.clone()],
        drafts: vec![DraftSession {
            id: "pending".into(),
            app_session_id: 1,
            project: project.clone(),
            created_ms: 1,
            submitted: false,
            session_path: None,
            title: None,
        }],
    })?;
    let summary = SessionSummary::from_cached(
        "pi-session".into(),
        session.canonicalize()?,
        project.canonicalize()?,
        "Session".into(),
        "hello".into(),
        "2026-08-15T00:00:00Z".into(),
        None,
        SystemTime::now(),
        1,
        UsageSummary::default(),
        false,
        false,
        "session hello".into(),
    );
    store.replace_sessions(std::slice::from_ref(&summary))?;
    assert_ne!(store.cached_sessions("")?[0].app_session_id, 1);
    let outbox = store.enqueue_prompt(
        "draft:pending",
        &project,
        None,
        PromptMode::Normal,
        "hello",
        &[],
    )?;
    store.begin_prompt(outbox)?;
    store.complete_prompt(outbox, "draft:pending", Some(&session))?;

    assert!(store.queued_prompts()?.is_empty());
    let draft = &store.load_registry()?.drafts[0];
    assert!(draft.submitted);
    assert_eq!(draft.session_path, Some(session.canonicalize()?));
    assert_eq!(store.cached_sessions("")?[0].app_session_id, 1);

    store.replace_sessions(&[summary])?;
    assert_eq!(store.cached_sessions("")?[0].app_session_id, 1);
    Ok(())
}

#[test]
fn partial_session_index_updates_do_not_delete_omitted_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let paths = [temp.path().join("one.jsonl"), temp.path().join("two.jsonl")];
    for path in &paths {
        fs::write(path, "{}")?;
    }
    let summary = |id: &str, path: &std::path::Path| {
        SessionSummary::from_cached(
            id.into(),
            path.canonicalize().expect("session path"),
            project.canonicalize().expect("project path"),
            id.into(),
            String::new(),
            String::new(),
            None,
            SystemTime::now(),
            0,
            UsageSummary::default(),
            false,
            false,
            id.into(),
        )
    };
    let sessions = [summary("one", &paths[0]), summary("two", &paths[1])];
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    store.replace_sessions(&sessions)?;
    store.index_sessions(&sessions[..1], false)?;

    assert_eq!(store.cached_sessions("")?.len(), 2);
    Ok(())
}

#[test]
fn schema_v1_migrates_to_v6_with_defaults_and_outbox_preserved()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let database = temp.path().join("gui.sqlite3");
    seed_legacy_database(&database, 1, &project)?;

    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store.load_registry()?.drafts,
        vec![DraftSession {
            id: "legacy-draft".into(),
            app_session_id: 1,
            project: project.canonicalize()?,
            created_ms: 7,
            submitted: false,
            session_path: None,
            title: None,
        }]
    );
    let queued = store.queued_prompts()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message, "legacy prompt");
    assert!(queued[0].images.is_empty());
    drop(store);

    assert_eq!(database_schema_version(&database)?, 6);
    Ok(())
}

#[test]
fn schema_v2_migrates_to_v6_with_defaults_and_outbox_preserved()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let database = temp.path().join("gui.sqlite3");
    seed_legacy_database(&database, 2, &project)?;

    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store.load_registry()?.drafts,
        vec![DraftSession {
            id: "legacy-draft".into(),
            app_session_id: 1,
            project: project.canonicalize()?,
            created_ms: 7,
            submitted: false,
            session_path: None,
            title: None,
        }]
    );
    let queued = store.queued_prompts()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message, "legacy prompt");
    assert_eq!(
        queued[0].images,
        vec![PromptImage::new("aGVsbG8=".into(), "image/png".into())]
    );
    drop(store);

    assert_eq!(database_schema_version(&database)?, 6);
    Ok(())
}

#[test]
fn schema_v3_migrates_to_v6_with_running_default() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let database = temp.path().join("gui.sqlite3");
    seed_legacy_database(&database, 2, &project)?;
    Connection::open(&database)?.execute_batch(
        "ALTER TABLE drafts ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE drafts ADD COLUMN session_path TEXT;
         UPDATE meta SET value='3' WHERE key='schema_version';",
    )?;

    let store = StateStore::open_at(&database)?;
    assert_eq!(database_schema_version(&database)?, 6);
    assert!(store.cached_sessions("")?.is_empty());
    Ok(())
}

#[test]
fn schema_v4_migrates_to_v6_with_provisional_title_default()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let database = temp.path().join("gui.sqlite3");
    seed_legacy_database(&database, 2, &project)?;
    Connection::open(&database)?.execute_batch(
        "ALTER TABLE drafts ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE drafts ADD COLUMN session_path TEXT;
         ALTER TABLE sessions ADD COLUMN is_running INTEGER NOT NULL DEFAULT 0;
         UPDATE meta SET value='4' WHERE key='schema_version';",
    )?;

    let store = StateStore::open_at(&database)?;
    assert_eq!(database_schema_version(&database)?, 6);
    assert_eq!(store.load_registry()?.drafts[0].title, None);
    Ok(())
}

#[test]
fn schema_v5_migrates_existing_sessions_and_drafts_to_incremental_ids()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let database = temp.path().join("gui.sqlite3");
    seed_legacy_database(&database, 2, &project)?;
    let connection = Connection::open(&database)?;
    connection.execute_batch(
        "ALTER TABLE drafts ADD COLUMN submitted INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE drafts ADD COLUMN session_path TEXT;
         ALTER TABLE sessions ADD COLUMN is_running INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE drafts ADD COLUMN provisional_title TEXT;
         UPDATE meta SET value='5' WHERE key='schema_version';",
    )?;
    connection.execute(
        "INSERT INTO sessions(
           path, id, project, title, first_user_message, timestamp, parent_session,
           modified_ms, file_size, message_count, input_tokens, output_tokens,
           cache_read_tokens, cache_write_tokens, total_tokens, cost_micros,
           search_text, settled_ms, is_running
         ) VALUES(
           '/legacy-session.jsonl', 'pi-legacy', ?1, 'Legacy', '', '', NULL,
           1, 0, 0, 0, 0, 0, 0, 0, 0, 'legacy', NULL, 0
         )",
        [project.to_string_lossy()],
    )?;
    drop(connection);

    let store = StateStore::open_at(&database)?;
    let draft = &store.load_registry()?.drafts[0];
    let session = &store.cached_sessions("")?[0];

    assert!(draft.app_session_id > 0);
    assert!(session.app_session_id > 0);
    assert_ne!(draft.app_session_id, session.app_session_id);
    assert_eq!(database_schema_version(&database)?, 6);
    Ok(())
}

#[test]
fn relocating_session_paths_preserves_application_identity_and_composer_state()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let source_project = temp.path().join("source");
    let target_project = temp.path().join("target");
    fs::create_dir(&source_project)?;
    fs::create_dir(&target_project)?;
    let source = temp.path().join("source.jsonl");
    let target = temp.path().join("target.jsonl");
    drop(StateStore::open_at(&database)?);
    let connection = Connection::open(&database)?;
    connection.execute(
        "INSERT INTO app_sessions(id, session_path, created_ms) VALUES(42, ?1, 1)",
        [source.to_string_lossy()],
    )?;
    connection.execute(
        "INSERT INTO sessions(
           path, id, project, title, first_user_message, timestamp, parent_session,
           modified_ms, file_size, message_count, input_tokens, output_tokens,
           cache_read_tokens, cache_write_tokens, total_tokens, cost_micros,
           search_text, settled_ms, is_running, app_session_id
         ) VALUES(?1, 'pi-id', ?2, 'Title', '', '', NULL, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                  'title', 7, 0, 42)",
        params![source.to_string_lossy(), source_project.to_string_lossy()],
    )?;
    connection.execute(
        "INSERT INTO composer_sessions(
           target, text, cursor, selection_start, selection_end, history_json, updated_ms
         ) VALUES(?1, 'draft', 5, 5, 5, '[]', 1)",
        [format!("session:{}", source.display())],
    )?;
    drop(connection);

    StateStore::open_at(&database)?
        .relocate_session_paths(&[(source.clone(), target.clone())], &target_project)?;

    let connection = Connection::open(&database)?;
    assert_eq!(
        connection.query_row(
            "SELECT id FROM app_sessions WHERE session_path=?1",
            [target.to_string_lossy()],
            |row| row.get::<_, i64>(0),
        )?,
        42
    );
    assert_eq!(
        connection.query_row(
            "SELECT project FROM sessions WHERE path=?1",
            [target.to_string_lossy()],
            |row| row.get::<_, String>(0),
        )?,
        target_project.to_string_lossy()
    );
    assert_eq!(
        connection.query_row(
            "SELECT text FROM composer_sessions WHERE target=?1",
            [format!("session:{}", target.display())],
            |row| row.get::<_, String>(0),
        )?,
        "draft"
    );
    Ok(())
}

#[test]
fn submitted_draft_without_session_path_survives_reopen() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    store.save_registry(&Registry {
        projects: vec![project.clone()],
        drafts: vec![DraftSession {
            id: "pending".into(),
            app_session_id: 1,
            project,
            created_ms: 1,
            submitted: true,
            session_path: None,
            title: Some("Pending session".into()),
        }],
    })?;
    drop(store);

    let store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    let registry = store.load_registry()?;
    assert_eq!(registry.drafts.len(), 1);
    assert!(registry.drafts[0].submitted);
    assert_eq!(registry.drafts[0].session_path, None);
    assert_eq!(registry.drafts[0].title.as_deref(), Some("Pending session"));
    Ok(())
}

fn seed_legacy_database(
    database: &std::path::Path,
    version: i64,
    project: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::open(database)?;
    connection.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE drafts (
           id TEXT PRIMARY KEY,
           project TEXT NOT NULL,
           created_ms INTEGER NOT NULL
         );
         CREATE TABLE sessions (
           path TEXT PRIMARY KEY,
           id TEXT NOT NULL,
           project TEXT NOT NULL,
           title TEXT NOT NULL,
           first_user_message TEXT NOT NULL,
           timestamp TEXT NOT NULL,
           parent_session TEXT,
           modified_ms INTEGER NOT NULL,
           file_size INTEGER NOT NULL,
           message_count INTEGER NOT NULL,
           input_tokens INTEGER NOT NULL,
           output_tokens INTEGER NOT NULL,
           cache_read_tokens INTEGER NOT NULL,
           cache_write_tokens INTEGER NOT NULL,
           total_tokens INTEGER NOT NULL,
           cost_micros INTEGER NOT NULL,
           search_text TEXT NOT NULL,
           settled_ms INTEGER
         );",
    )?;
    connection.execute(
        "INSERT INTO meta(key, value) VALUES('schema_version', ?1)",
        [version],
    )?;
    connection.execute(
        "INSERT INTO drafts(id, project, created_ms) VALUES('legacy-draft', ?1, 7)",
        [project.to_string_lossy()],
    )?;
    if version == 1 {
        connection.execute_batch(
            "CREATE TABLE outbox (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               target TEXT NOT NULL,
               project TEXT NOT NULL,
               session_path TEXT,
               mode TEXT NOT NULL,
               message TEXT NOT NULL,
               state TEXT NOT NULL DEFAULT 'queued',
               created_ms INTEGER NOT NULL,
               error TEXT
             );",
        )?;
        connection.execute(
            "INSERT INTO outbox(target, project, mode, message, created_ms)
             VALUES('draft:legacy-draft', ?1, 'normal', 'legacy prompt', 8)",
            [project.to_string_lossy()],
        )?;
    } else {
        connection.execute_batch(
            "CREATE TABLE outbox (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               target TEXT NOT NULL,
               project TEXT NOT NULL,
               session_path TEXT,
               mode TEXT NOT NULL,
               message TEXT NOT NULL,
               state TEXT NOT NULL DEFAULT 'queued',
               created_ms INTEGER NOT NULL,
               error TEXT,
               images_json TEXT NOT NULL DEFAULT '[]'
             );",
        )?;
        let images =
            serde_json::to_string(&[PromptImage::new("aGVsbG8=".into(), "image/png".into())])?;
        connection.execute(
            "INSERT INTO outbox(
               target, project, mode, message, created_ms, images_json
             ) VALUES('draft:legacy-draft', ?1, 'normal', 'legacy prompt', 8, ?2)",
            params![project.to_string_lossy(), images],
        )?;
    }
    Ok(())
}

fn database_schema_version(database: &std::path::Path) -> rusqlite::Result<i64> {
    Connection::open(database)?.query_row(
        "SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'",
        [],
        |row| row.get(0),
    )
}

#[test]
fn concurrent_state_store_open_waits_for_schema_writers() -> Result<(), Box<dyn std::error::Error>>
{
    const OPENERS: usize = 8;

    let temp = tempdir()?;
    let database = Arc::new(temp.path().join("state/gui.sqlite3"));
    let barrier = Arc::new(Barrier::new(OPENERS));
    let handles = (0..OPENERS)
        .map(|_| {
            let database = database.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                StateStore::open_at(&database).map(drop)
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle
            .join()
            .map_err(|_| std::io::Error::other("state opener panicked"))?
            .map_err(std::io::Error::other)?;
    }
    Ok(())
}
