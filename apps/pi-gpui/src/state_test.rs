use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::SystemTime,
};

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
    let draft = DraftSession {
        id: "draft-one".into(),
        project: project.clone(),
        created_ms: 7,
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
        let session_path = temp.path().join("session.jsonl");
        fs::write(&session_path, "{}")?;
        store.replace_sessions(&[SessionSummary::from_cached(
            "session-one".into(),
            session_path.canonicalize()?,
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
        store.set_settled(&session_path.canonicalize()?, true)?;
    }
    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store.load_registry()?.drafts,
        vec![DraftSession {
            project: project.canonicalize()?,
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
