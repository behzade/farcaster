use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Barrier},
    thread,
    time::SystemTime,
};

use rusqlite::{Connection, params};
use tempfile::tempdir;

use crate::{
    agents::ConfigurationCatalog,
    app::infrastructure::persistence::{
        CachedConfigurationCatalog, ComposerRecord, StateStore, WindowPlacement, WindowState,
    },
    projects::{self, DraftSession, Registry},
    protocol::{Model, PromptImage, PromptMode},
    sessions::{SessionSummary, UsageSummary},
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
fn configuration_catalogs_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let cached = CachedConfigurationCatalog {
        harness: "codex-cli".into(),
        project: temp.path().to_path_buf(),
        catalog: ConfigurationCatalog {
            models: vec![Model {
                id: "model".into(),
                name: "Model".into(),
                provider: "provider".into(),
                context_window: 200_000,
                reasoning: true,
            }],
            efforts: vec!["high".into()],
        },
    };

    StateStore::open_at(&database)?.save_configuration_catalogs(&[cached.clone()])?;

    assert_eq!(
        StateStore::open_at(&database)?.load_configuration_catalogs()?,
        vec![cached]
    );
    Ok(())
}

#[test]
fn network_proxy_round_trips_and_clears() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.load_network_proxy()?, None);

    store.save_network_proxy(Some("http://proxy.example:8080"))?;
    assert_eq!(
        StateStore::open_at(&database)?
            .load_network_proxy()?
            .as_deref(),
        Some("http://proxy.example:8080")
    );

    store.save_network_proxy(None)?;
    assert_eq!(store.load_network_proxy()?, None);
    assert!(
        store
            .save_network_proxy(Some("socks5://proxy.example"))
            .is_err()
    );
    Ok(())
}

#[test]
fn repository_backend_preferences_default_to_empty() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    assert!(store.load_repository_backend_preferences()?.is_empty());
    Ok(())
}

#[test]
fn repository_backend_preferences_round_trip_deterministically()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let alpha = temp.path().join("alpha");
    let zeta = temp.path().join("zeta");
    fs::create_dir(&alpha)?;
    fs::create_dir(&zeta)?;
    let preferences = BTreeMap::from([
        (zeta.canonicalize()?, "jj".to_owned()),
        (alpha.canonicalize()?, "git".to_owned()),
    ]);

    StateStore::open_at(&database)?.save_repository_backend_preferences(&preferences)?;

    assert_eq!(
        StateStore::open_at(&database)?.load_repository_backend_preferences()?,
        preferences
    );
    let stored = Connection::open(&database)?.query_row(
        "SELECT value FROM meta WHERE key='repository_backend_preferences'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    assert_eq!(stored, serde_json::to_string(&preferences)?);
    assert!(stored.find("alpha") < stored.find("zeta"));

    fs::remove_dir_all(&alpha)?;
    StateStore::open_at(&database)?.save_repository_backend_preferences(&preferences)?;
    assert_eq!(
        StateStore::open_at(&database)?.load_repository_backend_preferences()?,
        preferences
    );
    Ok(())
}

#[test]
fn repository_backend_preferences_reject_unknown_and_malformed_values()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let project = project.canonicalize()?;
    let store = StateStore::open_at(&database)?;
    let unknown = BTreeMap::from([(project.clone(), "svn".to_owned())]);
    let Err(error) = store.save_repository_backend_preferences(&unknown) else {
        return Err("unknown repository backend was accepted".into());
    };
    assert!(error.contains("unknown repository backend preference"));
    drop(store);

    let connection = Connection::open(&database)?;
    connection.execute(
        "INSERT INTO meta(key, value) VALUES('repository_backend_preferences', ?1)",
        ["not json"],
    )?;
    drop(connection);
    let Err(error) = StateStore::open_at(&database)?.load_repository_backend_preferences() else {
        return Err("malformed repository backend preferences were accepted".into());
    };
    assert!(error.contains("decode repository backend preferences"));
    Ok(())
}

#[test]
fn legacy_pi_gpui_v7_state_import_restores_archives_once() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempdir()?;
    let project = temp.path().join("project");
    let session_path = temp.path().join("session.jsonl");
    fs::create_dir(&project)?;
    fs::write(&session_path, "{}")?;
    let project = project.canonicalize()?;
    let session_path = session_path.canonicalize()?;
    let session = SessionSummary::from_cached(
        "session-one".into(),
        session_path.clone(),
        project,
        "title".into(),
        "hello".into(),
        "2026-08-15T00:00:00Z".into(),
        None,
        SystemTime::now(),
        1,
        UsageSummary::default(),
        false,
        false,
        "title hello".into(),
    );
    let legacy_path = temp.path().join("gui-state.sqlite3");
    let destination_path = temp.path().join("state.sqlite3");
    {
        let mut legacy = StateStore::open_at(&legacy_path)?;
        legacy.replace_sessions(std::slice::from_ref(&session))?;
        legacy.set_session_archived(&session_path, true)?;
        legacy.save_composer_session(&ComposerRecord {
            target: format!("session:{}", session_path.display()),
            text: "legacy draft".into(),
            ..ComposerRecord::default()
        })?;
    }
    Connection::open(&legacy_path)?.execute_batch(
        "ALTER TABLE sessions DROP COLUMN harness;
         UPDATE meta SET value='7' WHERE key='schema_version';",
    )?;
    let mut destination = StateStore::open_at(&destination_path)?;
    destination.replace_sessions(std::slice::from_ref(&session))?;

    destination.import_legacy_pi_gpui_state(&legacy_path)?;

    assert!(destination.cached_sessions("")?[0].archived);
    assert_eq!(
        destination.load_composer_sessions()?[0].text,
        "legacy draft"
    );
    destination.set_session_archived(&session_path, false)?;
    destination.import_legacy_pi_gpui_state(&legacy_path)?;
    assert!(!destination.cached_sessions("")?[0].archived);
    Ok(())
}

#[test]
fn registry_composer_and_outbox_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    let removed_project = temp.path().join("removed-project");
    fs::create_dir(&project)?;
    fs::create_dir(&removed_project)?;
    let database = temp.path().join("state/gui.sqlite3");
    let session_path = temp.path().join("submitted-session.jsonl");
    let catalog_session_path = temp.path().join("session.jsonl");
    fs::write(&session_path, "{}")?;
    let draft = DraftSession {
        id: "draft-one".into(),
        app_session_id: 1,
        harness: "pi".into(),
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
            excluded_projects: vec![removed_project.canonicalize()?],
            drafts: vec![draft.clone()],
        })?;
        store.save_app_session_order(&[7, 3, 1])?;
        store.enqueue_prompt_with_presentation(
            "draft:draft-one",
            "pi",
            &project,
            None,
            PromptMode::Normal,
            "expanded prompt",
            Some("$commit hello"),
            Some("expanded prompt"),
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
        store.set_session_archived(&catalog_session_path.canonicalize()?, false)?;
    }
    let mut store = StateStore::open_at(&database)?;
    let registry = store.load_registry()?;
    assert_eq!(
        registry.excluded_projects,
        vec![removed_project.canonicalize()?]
    );
    assert_eq!(
        registry.drafts,
        vec![DraftSession {
            project: project.canonicalize()?,
            session_path: Some(session_path.canonicalize()?),
            ..draft
        }]
    );
    assert_eq!(store.load_app_session_order()?, vec![7, 3, 1]);
    let queued = store.queued_prompts()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].harness, "pi");
    assert_eq!(queued[0].message, "expanded prompt");
    assert_eq!(queued[0].display_message.as_deref(), Some("$commit hello"));
    assert_eq!(queued[0].invocation.as_deref(), Some("expanded prompt"));
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
    assert!(!store.cached_sessions("")?[0].archived);
    assert!(store.cached_sessions("")?[0].is_running);
    store.set_session_archived(&catalog_session_path.canonicalize()?, true)?;
    assert!(store.cached_sessions("")?[0].archived);
    store.set_session_archived(&catalog_session_path.canonicalize()?, false)?;
    assert!(!store.cached_sessions("")?[0].archived);
    store.begin_prompt(queued[0].id)?;
    store.complete_prompt(
        queued[0].id,
        "draft:draft-one",
        Some(&session_path.canonicalize()?),
    )?;
    assert!(store.queued_prompts()?.is_empty());
    assert_eq!(
        store.prompt_presentations(&session_path.canonicalize()?)?,
        vec![crate::agents::PromptPresentation {
            resolved_message: "expanded prompt".into(),
            display_message: "$commit hello".into(),
            invocation: "expanded prompt".into(),
        }]
    );
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
fn session_harness_survives_the_cache() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    let session_path = temp.path().join("session.jsonl");
    fs::create_dir(&project)?;
    fs::write(&session_path, "{}")?;
    let mut session = SessionSummary::from_cached(
        "session".into(),
        session_path,
        project,
        "Session".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    session.harness = "codex-cli".into();
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    store.replace_sessions(&[session])?;

    assert_eq!(store.cached_sessions("")?[0].harness, "codex-cli");
    Ok(())
}

#[test]
fn imported_sessions_are_active_while_recent_even_without_running_status()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let now = SystemTime::now();
    let summaries = [
        ("recent-running", now, true),
        ("recent-done", now, false),
        (
            "old-running",
            now - std::time::Duration::from_secs(4 * 60 * 60),
            true,
        ),
    ]
    .into_iter()
    .map(|(id, modified, is_running)| {
        let path = temp.path().join(format!("{id}.jsonl"));
        fs::write(&path, "{}")?;
        Ok::<_, std::io::Error>(SessionSummary::from_cached(
            id.into(),
            path,
            project.clone(),
            id.into(),
            String::new(),
            String::new(),
            None,
            modified,
            0,
            UsageSummary::default(),
            false,
            is_running,
            id.into(),
        ))
    })
    .collect::<Result<Vec<_>, _>>()?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    store.replace_sessions(&summaries)?;

    let sessions = store.cached_sessions("")?;
    let archived = sessions
        .iter()
        .map(|session| (session.id.as_str(), session.archived))
        .collect::<std::collections::HashMap<_, _>>();
    assert!(!archived["recent-running"]);
    assert!(!archived["recent-done"]);
    assert!(archived["old-running"]);
    Ok(())
}

#[test]
fn import_classification_is_not_reapplied_when_a_session_finishes()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    let path = temp.path().join("session.jsonl");
    fs::create_dir(&project)?;
    fs::write(&path, "{}")?;
    let summary = |is_running| {
        SessionSummary::from_cached(
            "session".into(),
            path.clone(),
            project.clone(),
            "Session".into(),
            String::new(),
            String::new(),
            None,
            SystemTime::now(),
            0,
            UsageSummary::default(),
            false,
            is_running,
            String::new(),
        )
    };
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    store.replace_sessions(&[summary(true)])?;
    assert!(!store.cached_sessions("")?[0].archived);
    store.replace_sessions(&[summary(false)])?;

    assert!(!store.cached_sessions("")?[0].archived);
    Ok(())
}

#[test]
fn draft_harness_survives_the_registry() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let mut draft = DraftSession::new("draft".into(), 1, project.clone(), 1);
    assert!(draft.change_harness("opencode2".into()));
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;

    projects::save_registry(
        &mut store,
        &Registry {
            projects: vec![project],
            excluded_projects: Vec::new(),
            drafts: vec![draft],
        },
    )?;

    assert_eq!(
        projects::load_registry(&store)?.drafts[0].harness,
        "opencode2"
    );
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
        excluded_projects: Vec::new(),
        drafts: vec![DraftSession {
            id: "pending".into(),
            app_session_id: 1,
            harness: "pi".into(),
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
        "pi",
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
fn schema_v1_migrates_to_v11_with_defaults_and_outbox_preserved()
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
            harness: "pi".into(),
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
    assert_eq!(queued[0].display_message, None);
    assert_eq!(queued[0].invocation, None);
    assert!(queued[0].images.is_empty());
    drop(store);

    assert_eq!(database_schema_version(&database)?, 11);
    Ok(())
}

#[test]
fn schema_v2_migrates_to_v11_with_defaults_and_outbox_preserved()
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
            harness: "pi".into(),
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
    assert_eq!(queued[0].display_message, None);
    assert_eq!(queued[0].invocation, None);
    assert_eq!(
        queued[0].images,
        vec![PromptImage::new("aGVsbG8=".into(), "image/png".into())]
    );
    drop(store);

    assert_eq!(database_schema_version(&database)?, 11);
    Ok(())
}

#[test]
fn schema_v3_migrates_to_v11_with_running_default() -> Result<(), Box<dyn std::error::Error>> {
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
    assert_eq!(database_schema_version(&database)?, 11);
    assert!(store.cached_sessions("")?.is_empty());
    Ok(())
}

#[test]
fn schema_v4_migrates_to_v11_with_provisional_title_default()
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
    assert_eq!(database_schema_version(&database)?, 11);
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
    assert_eq!(session.harness, "pi");
    assert_eq!(database_schema_version(&database)?, 11);
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
fn deleting_session_state_removes_the_family_and_preserves_other_sessions()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let project = temp.path().join("project");
    let root = temp.path().join("root.jsonl");
    let child = temp.path().join("child.jsonl");
    let other = temp.path().join("other.jsonl");
    fs::create_dir(&project)?;
    drop(StateStore::open_at(&database)?);
    let connection = Connection::open(&database)?;
    for (index, path) in [&root, &child, &other].into_iter().enumerate() {
        let app_session_id = index as i64 + 1;
        let id = format!("session-{index}");
        let draft_id = format!("draft-{index}");
        connection.execute(
            "INSERT INTO app_sessions(id, draft_id, session_path, created_ms)
             VALUES(?1, ?2, ?3, 1)",
            params![app_session_id, draft_id, path.to_string_lossy()],
        )?;
        connection.execute(
            "INSERT INTO sessions(
               path, id, project, title, first_user_message, timestamp, parent_session,
               modified_ms, file_size, message_count, input_tokens, output_tokens,
               cache_read_tokens, cache_write_tokens, total_tokens, cost_micros,
               search_text, settled_ms, is_running, app_session_id
             ) VALUES(?1, ?2, ?3, 'Title', '', '', NULL, 1, 0, 0, 0, 0, 0, 0, 0, 0,
                      'title', 1, 0, ?4)",
            params![
                path.to_string_lossy(),
                id,
                project.to_string_lossy(),
                app_session_id
            ],
        )?;
        connection.execute(
            "INSERT INTO drafts(
               id, project, created_ms, submitted, session_path, provisional_title,
               app_session_id
             ) VALUES(?1, ?2, 1, 1, ?3, NULL, ?4)",
            params![
                draft_id,
                project.to_string_lossy(),
                path.to_string_lossy(),
                app_session_id
            ],
        )?;
        connection.execute(
            "INSERT INTO outbox(
               target, project, session_path, mode, message, state, created_ms, images_json
             ) VALUES(?1, ?2, ?3, 'normal', 'failed prompt', 'failed', 1, '[]')",
            params![
                format!("session:{}", path.display()),
                project.to_string_lossy(),
                path.to_string_lossy()
            ],
        )?;
        connection.execute(
            "INSERT INTO composer_sessions(
               target, text, cursor, selection_start, selection_end, history_json, updated_ms
             ) VALUES(?1, 'draft', 5, 5, 5, '[]', 1)",
            [format!("session:{}", path.display())],
        )?;
    }
    drop(connection);

    StateStore::open_at(&database)?.delete_session_state(&[root.clone(), child.clone()])?;

    let connection = Connection::open(&database)?;
    for path in [&root, &child] {
        let path = path.to_string_lossy();
        let target = format!("session:{path}");
        for (table, column, value) in [
            ("app_sessions", "session_path", path.as_ref()),
            ("sessions", "path", path.as_ref()),
            ("drafts", "session_path", path.as_ref()),
            ("outbox", "session_path", path.as_ref()),
            ("composer_sessions", "target", target.as_str()),
        ] {
            let count = connection.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1"),
                [value],
                |row| row.get::<_, i64>(0),
            )?;
            assert_eq!(count, 0, "{table} retained deleted session state");
        }
    }
    let other_path = other.to_string_lossy();
    for (table, column, value) in [
        ("app_sessions", "session_path", other_path.as_ref()),
        ("sessions", "path", other_path.as_ref()),
        ("drafts", "session_path", other_path.as_ref()),
        ("outbox", "session_path", other_path.as_ref()),
    ] {
        let count = connection.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1"),
            [value],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(count, 1, "{table} lost unrelated session state");
    }
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(*) FROM composer_sessions WHERE target=?1",
            [format!("session:{other_path}")],
            |row| row.get::<_, i64>(0),
        )?,
        1
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
        excluded_projects: Vec::new(),
        drafts: vec![DraftSession {
            id: "pending".into(),
            app_session_id: 1,
            harness: "pi".into(),
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
