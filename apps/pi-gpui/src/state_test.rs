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
    state::{ComposerRecord, StateStore},
};

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
        project: project.clone(),
        created_ms: 7,
        submitted: true,
        session_path: Some(session_path.clone()),
    };
    {
        let mut store = StateStore::open_at(&database)?;
        store.save_registry(&Registry {
            projects: vec![project.clone()],
            drafts: vec![draft.clone()],
        })?;
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
            "literal_100% hello".into(),
        )])?;
        store.set_settled(&catalog_session_path.canonicalize()?, true)?;
    }
    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store.load_registry()?.drafts,
        vec![DraftSession {
            project: project.canonicalize()?,
            session_path: Some(session_path.canonicalize()?),
            ..draft
        }]
    );
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
    store.begin_prompt(queued[0].id)?;
    assert!(store.queued_prompts()?.is_empty());
    store.delete_composer_session("draft:draft-one")?;
    assert!(store.load_composer_sessions()?.is_empty());
    Ok(())
}

#[test]
fn schema_v1_migrates_to_v3_with_defaults_and_outbox_preserved()
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
            project: project.canonicalize()?,
            created_ms: 7,
            submitted: false,
            session_path: None,
        }]
    );
    let queued = store.queued_prompts()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message, "legacy prompt");
    assert!(queued[0].images.is_empty());
    drop(store);

    assert_eq!(database_schema_version(&database)?, 3);
    Ok(())
}

#[test]
fn schema_v2_migrates_to_v3_with_defaults_and_outbox_preserved()
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
            project: project.canonicalize()?,
            created_ms: 7,
            submitted: false,
            session_path: None,
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

    assert_eq!(database_schema_version(&database)?, 3);
    Ok(())
}

#[test]
fn state_store_rejects_submitted_draft_without_session_path()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    let error = store
        .save_registry(&Registry {
            projects: vec![project.clone()],
            drafts: vec![DraftSession {
                id: "invalid".into(),
                project,
                created_ms: 1,
                submitted: true,
                session_path: None,
            }],
        })
        .expect_err("submitted draft without a path must not be persisted");

    assert!(error.contains("submitted draft has no session path"));
    assert_eq!(store.load_registry()?, Registry::default());
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
