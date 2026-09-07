use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
    time::SystemTime,
};

use rusqlite::{Connection, params};
use tempfile::tempdir;

use crate::{
    agents::ConfigurationCatalog,
    app::infrastructure::persistence::{
        CachedConfigurationCatalog, CachedSessionControlDefaults, ComposerRecord, StateStore,
        WindowPlacement, WindowState,
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
                efforts: Some(vec!["low".into(), "high".into()]),
            }],
            efforts: vec!["high".into()],
        },
    };

    StateStore::open_at(&database)?.save_configuration_catalogs(std::slice::from_ref(&cached))?;

    assert_eq!(
        StateStore::open_at(&database)?.load_configuration_catalogs()?,
        vec![cached]
    );
    Ok(())
}

#[test]
fn session_control_defaults_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let cached = CachedSessionControlDefaults {
        harness: "codex-cli".into(),
        model: Some(Model {
            id: "model".into(),
            name: "Model".into(),
            provider: "provider".into(),
            context_window: 200_000,
            reasoning: true,
            efforts: Some(vec!["low".into(), "high".into()]),
        }),
        effort: Some("high".into()),
    };

    StateStore::open_at(&database)?.save_session_control_defaults(std::slice::from_ref(&cached))?;

    assert_eq!(
        StateStore::open_at(&database)?.load_session_control_defaults()?,
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
fn application_settings_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.load_application_modifier()?, None);
    assert!(store.load_builtin_mcp_enabled()?);

    store.save_application_settings("ctrl", Some("http://proxy.example:8080"))?;
    store.save_builtin_mcp_enabled(false)?;
    assert_eq!(
        StateStore::open_at(&database)?
            .load_application_modifier()?
            .as_deref(),
        Some("ctrl")
    );
    assert_eq!(
        StateStore::open_at(&database)?
            .load_network_proxy()?
            .as_deref(),
        Some("http://proxy.example:8080")
    );
    assert!(!StateStore::open_at(&database)?.load_builtin_mcp_enabled()?);
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
    let stored = Connection::open(&database)?
        .prepare("SELECT path, repository_backend FROM projects WHERE repository_backend IS NOT NULL ORDER BY path")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        stored,
        vec![
            (
                alpha.canonicalize()?.to_string_lossy().into_owned(),
                "git".into()
            ),
            (
                zeta.canonicalize()?.to_string_lossy().into_owned(),
                "jj".into()
            ),
        ]
    );

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
        "INSERT INTO projects(path, added_ms, repository_backend) VALUES(?1, 1, 'not-a-backend')
         ON CONFLICT(path) DO UPDATE SET repository_backend='not-a-backend'",
        [project.to_string_lossy()],
    )?;
    drop(connection);
    let Err(error) = StateStore::open_at(&database)?.load_repository_backend_preferences() else {
        return Err("malformed repository backend preferences were accepted".into());
    };
    assert!(error.contains("unknown repository backend preference"));
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
    seed_legacy_database(&legacy_path, 7, &session.project)?;
    let legacy = Connection::open(&legacy_path)?;
    legacy.execute_batch(
        "CREATE TABLE projects(path TEXT PRIMARY KEY, added_ms INTEGER NOT NULL);
         CREATE TABLE composer_sessions(target TEXT PRIMARY KEY, text TEXT NOT NULL,
           cursor INTEGER NOT NULL, selection_start INTEGER NOT NULL, selection_end INTEGER NOT NULL,
           history_json TEXT NOT NULL, updated_ms INTEGER NOT NULL);"
    )?;
    legacy.execute(
        "INSERT INTO projects VALUES(?1, 1)",
        [session.project.to_string_lossy()],
    )?;
    legacy.execute(
        "INSERT INTO sessions VALUES(?1, 'session-one', ?2, 'title', 'hello', '', NULL,
          1, 0, 1, 0, 0, 0, 0, 0, 0, 'title hello', 1)",
        params![
            session_path.to_string_lossy(),
            session.project.to_string_lossy()
        ],
    )?;
    legacy.execute(
        "INSERT INTO composer_sessions VALUES(?1, 'legacy draft', 0, 0, 0, '[]', 1)",
        [format!("session:{}", session_path.display())],
    )?;
    drop(legacy);
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
            target: format!("session:{}", session_path.canonicalize()?.display()),
            text: "draft text".into(),
            cursor: 6,
            selection_start: 2,
            selection_end: 6,
            history: vec!["new".into(), "old".into()],
        }]
    );
    assert_eq!(store.cached_sessions("literal_100%")?.len(), 1);
    assert!(!store.cached_sessions("")?[0].archived);
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

    let mut draft = DraftSession::new("first".into(), 0, temp.path().to_path_buf(), 1);
    draft.harness = "codex-cli".into();
    let first = store.allocate_app_session_id(&draft)?;
    draft.id = "second".into();
    let second = store.allocate_app_session_id(&draft)?;

    assert!(first > 0);
    assert_eq!(second, first + 1);
    let registry = store.load_registry()?;
    assert_eq!(registry.projects, vec![temp.path().to_path_buf()]);
    assert!(
        registry
            .drafts
            .iter()
            .all(|draft| draft.harness == "codex-cli")
    );
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
        .map(|session| (session.title.as_str(), session.archived))
        .collect::<std::collections::HashMap<_, _>>();
    assert!(!archived["recent-running"]);
    assert!(!archived["recent-done"]);
    assert!(!archived["old-running"]);
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
    store.save_composer_session(&ComposerRecord {
        target: "draft:pending".into(),
        text: "draft text".into(),
        ..ComposerRecord::default()
    })?;
    store.save_composer_session(&ComposerRecord {
        target: format!("session:{}", session.display()),
        text: "newer text".into(),
        ..ComposerRecord::default()
    })?;
    let connection = Connection::open(temp.path().join("gui.sqlite3"))?;
    connection.execute(
        "UPDATE composer_sessions SET updated_ms=CASE WHEN session_id=1 THEN 1 ELSE 2 END",
        [],
    )?;
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
    assert_eq!(store.load_composer_sessions()?.len(), 1);
    assert_eq!(store.load_composer_sessions()?[0].text, "newer text");
    let registry = store.load_registry()?;
    assert_eq!(registry.drafts[0].app_session_id, 1);
    store.save_registry(&registry)?;

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

    assert_eq!(database_schema_version(&database)?, 12);
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

    assert_eq!(database_schema_version(&database)?, 12);
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
    assert_eq!(database_schema_version(&database)?, 12);
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
    assert_eq!(database_schema_version(&database)?, 12);
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
    assert_eq!(database_schema_version(&database)?, 12);
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
    fs::write(&source, "{}")?;
    let mut session = SessionSummary::from_cached(
        "pi-id".into(),
        source.clone(),
        source_project.clone(),
        "Title".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        UsageSummary::default(),
        false,
        false,
        "title".into(),
    );
    session.app_session_id = 42;
    let mut store = StateStore::open_at(&database)?;
    store.replace_sessions(std::slice::from_ref(&session))?;
    store.save_composer_session(&ComposerRecord {
        target: format!("session:{}", source.display()),
        text: "draft".into(),
        cursor: 5,
        selection_start: 5,
        selection_end: 5,
        history: Vec::new(),
    })?;
    let original_id = store.cached_sessions("")?[0].app_session_id;
    store.relocate_session_paths(&[(source.clone(), target.clone())], &target_project)?;

    let relocated = store.cached_sessions("")?;
    assert_eq!(relocated.len(), 1);
    assert_eq!(relocated[0].app_session_id, original_id);
    assert_eq!(
        relocated[0].path,
        crate::sessions::normalize_session_path(&target)
    );
    assert_eq!(
        relocated[0].project,
        target_project.canonicalize().unwrap_or(target_project)
    );
    assert_eq!(store.load_composer_sessions()?[0].text, "draft");
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
    let mut store = StateStore::open_at(&database)?;
    let mut sessions = Vec::new();
    for path in [&root, &child, &other] {
        fs::write(path, "{}")?;
        sessions.push(SessionSummary::from_cached(
            path.file_stem().unwrap().to_string_lossy().into_owned(),
            path.clone(),
            project.clone(),
            "Title".into(),
            String::new(),
            String::new(),
            None,
            SystemTime::now(),
            0,
            UsageSummary::default(),
            false,
            false,
            "title".into(),
        ));
    }
    store.replace_sessions(&sessions)?;
    for path in [&root, &child, &other] {
        store.save_composer_session(&ComposerRecord {
            target: format!("session:{}", path.display()),
            text: "draft".into(),
            cursor: 5,
            selection_start: 5,
            selection_end: 5,
            history: Vec::new(),
        })?;
    }
    store.delete_session_state(&[root.clone(), child.clone()])?;

    let remaining = store.cached_sessions("")?;
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].path,
        crate::sessions::normalize_session_path(&other)
    );
    let composers = store.load_composer_sessions()?;
    assert_eq!(composers.len(), 1);
    assert!(composers[0].target.contains("other.jsonl"));
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

#[test]
fn parent_identity_survives_child_first_and_partial_indexing()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let mut child = persistence_summary(temp.path(), "child");
    child.parent_session = Some("root".into());
    let root = persistence_summary(temp.path(), "root");
    let mut unrelated = persistence_summary(temp.path(), "other");
    unrelated.id = root.id.clone();
    unrelated.harness = "codex".into();
    store.index_sessions(&[child.clone(), unrelated], false)?;
    store.index_sessions(std::slice::from_ref(&root), false)?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    let cached = store.cached_sessions("")?;
    let cached_child = cached.iter().find(|s| s.path == child.path).unwrap();
    assert_eq!(cached_child.id, "child");
    assert_eq!(cached_child.parent_session.as_deref(), Some("root"));
    let connection = Connection::open(&database)?;
    let parent: (String, String) = connection.query_row(
        "SELECT p.backend_id, p.harness FROM sessions c JOIN sessions p ON p.id=c.parent_id
          WHERE c.backend_id='child'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(parent, ("root".into(), "pi".into()));
    Ok(())
}

#[test]
fn discovery_keeps_project_excluded_until_explicitly_restored()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().canonicalize()?;
    let database = temp.path().join("gui.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    store.save_registry(&Registry {
        projects: vec![],
        excluded_projects: vec![project.clone()],
        drafts: vec![],
    })?;
    store.index_sessions(&[persistence_summary(&project, "session")], false)?;
    drop(store);
    let mut store = StateStore::open_at(&database)?;
    let registry = store.load_registry()?;
    assert!(registry.projects.is_empty());
    assert_eq!(registry.excluded_projects, vec![project.clone()]);
    store.save_registry(&Registry {
        projects: vec![project.clone()],
        excluded_projects: vec![],
        drafts: vec![],
    })?;
    assert_eq!(store.load_registry()?.projects, vec![project]);
    assert!(store.load_registry()?.excluded_projects.is_empty());
    Ok(())
}

#[test]
fn discovery_prunes_only_disposable_catalog_rows() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    let sessions = ["disposable", "composer", "queued", "archive", "event"]
        .map(|id| persistence_summary(temp.path(), id));
    store.index_sessions(&sessions, false)?;
    store.save_composer_session(&ComposerRecord {
        target: format!("session:{}", sessions[1].path.display()),
        text: "unsent".into(),
        ..ComposerRecord::default()
    })?;
    store.enqueue_prompt(
        &format!("session:{}", sessions[2].path.display()),
        "pi",
        temp.path(),
        Some(&sessions[2].path),
        PromptMode::Normal,
        "queued",
        &[],
    )?;
    store.set_session_archived(&sessions[3].path, true)?;
    let mut foreign = sessions[3].clone();
    foreign.harness = "codex-cli".into();
    foreign.id = "foreign-archive".into();
    store.index_sessions(&[foreign], false)?;
    let presentation = store.enqueue_prompt_with_presentation(
        &format!("session:{}", sessions[4].path.display()),
        "pi",
        temp.path(),
        Some(&sessions[4].path),
        PromptMode::Normal,
        "expanded",
        Some("display"),
        Some("invocation"),
        &[],
    )?;
    store.complete_prompt(
        presentation,
        &format!("session:{}", sessions[4].path.display()),
        Some(&sessions[4].path),
    )?;
    store.index_sessions(&[], true)?;
    let retained = store
        .cached_sessions("")?
        .into_iter()
        .map(|s| s.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        retained,
        ["composer", "queued", "archive", "event"]
            .map(String::from)
            .into()
    );
    assert_eq!(store.queued_prompts()?.len(), 1);
    assert_eq!(store.load_composer_sessions()?[0].text, "unsent");
    Ok(())
}

#[test]
fn accepted_pathless_draft_retains_presentation_and_outbox_ids_do_not_repeat()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    let draft = DraftSession::new("pending".into(), 0, temp.path().to_path_buf(), 1);
    store.allocate_app_session_id(&draft)?;
    let first = store.enqueue_prompt_with_presentation(
        "draft:pending",
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "expanded",
        Some("display"),
        Some("invocation"),
        &[],
    )?;
    store.complete_prompt(first, "draft:pending", None)?;
    let mut registry = store.load_registry()?;
    assert!(registry.drafts[0].submitted);
    registry.drafts[0].session_path = Some(temp.path().join("bound.jsonl"));
    store.save_registry(&registry)?;
    assert_eq!(
        store.prompt_presentations(registry.drafts[0].session_path.as_ref().unwrap())?[0]
            .display_message,
        "display"
    );
    let second = store.enqueue_prompt(
        "draft:pending",
        "pi",
        temp.path(),
        None,
        PromptMode::Normal,
        "next",
        &[],
    )?;
    assert!(second > first);
    store.complete_prompt(first, "draft:pending", None)?;
    assert_eq!(store.queued_prompts()?[0].id, second);
    Ok(())
}

#[test]
fn worker_identity_binds_a_discovered_locator_without_creating_a_second_session()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    store.save_worker_family(&crate::agents::WorkerFamilyLink {
        project: temp.path().to_path_buf(),
        parent_backend: "pi".into(),
        parent_session: "parent".into(),
        child_backend: "codex-cli".into(),
        child_session: "child".into(),
        execution: None,
    })?;
    let mut child = persistence_summary(temp.path(), "child");
    child.harness = "codex-cli".into();
    store.index_sessions(
        &[persistence_summary(temp.path(), "parent"), child.clone()],
        false,
    )?;
    assert_eq!(store.cached_sessions("")?.len(), 2);
    let links = store.load_worker_families()?;
    assert_eq!(links[0].child_session, child.path.to_string_lossy());
    let connection = Connection::open(&database)?;
    assert_eq!(
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })?,
        0
    );
    Ok(())
}

fn persistence_summary(project: &std::path::Path, id: &str) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        project.join(format!("{id}.jsonl")),
        project.to_path_buf(),
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

#[test]
fn worker_tasks_customization_and_deletion_survive_reopen() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("settings.sqlite3");
    let store = StateStore::open_at(&database)?;
    let mut tasks = store.load_worker_tasks()?;
    assert_eq!(tasks.tasks.len(), 3);
    tasks.tasks[0].name = "audit".into();
    tasks.tasks[0].independent.harness = "codex-cli".into();
    tasks.tasks[0].independent.provider = "openai".into();
    tasks.tasks.remove(1);
    store.save_application_settings_with_workers("ctrl", None, Some(&tasks))?;
    assert_eq!(StateStore::open_at(&database)?.load_worker_tasks()?, tasks);
    tasks.tasks.clear();
    store.save_application_settings_with_workers("ctrl", None, Some(&tasks))?;
    assert!(
        StateStore::open_at(&database)?
            .load_worker_tasks()?
            .tasks
            .is_empty()
    );
    Ok(())
}

#[test]
fn invalid_worker_tasks_do_not_partially_save_application_settings() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let store = StateStore::open_at(&temp.path().join("settings.sqlite3"))?;
    store.save_application_settings("ctrl", None)?;
    let original = store.load_worker_tasks()?;
    let mut invalid = original.clone();
    invalid.tasks[0].guided.provider.clear();
    assert!(
        store
            .save_application_settings_with_workers("cmd", None, Some(&invalid))
            .is_err()
    );
    assert_eq!(store.load_application_modifier()?.as_deref(), Some("ctrl"));
    assert_eq!(store.load_worker_tasks()?, original);
    store.save_application_settings_with_workers("cmd", None, Some(&original))?;
    assert_eq!(store.load_application_modifier()?.as_deref(), Some("cmd"));
    Ok(())
}

#[test]
fn cross_harness_worker_families_survive_reopen() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("settings.sqlite3");
    let link = crate::agents::WorkerFamilyLink {
        project: temp.path().to_owned(),
        child_backend: "opencode2".into(),
        child_session: "child-session".into(),
        parent_backend: "pi".into(),
        parent_session: "/sessions/parent.jsonl".into(),
        execution: Some(crate::agents::WorkerExecution {
            harness: "opencode2".into(),
            provider: "opencode-go".into(),
            model: "glm-5.3-flash".into(),
            effort: None,
        }),
    };
    let mut store = StateStore::open_at(&database)?;
    store.save_worker_family(&link)?;
    store.save_worker_family(&link)?;
    assert_eq!(
        StateStore::open_at(&database)?.load_worker_families()?,
        vec![link.clone()]
    );
    let mut session = SessionSummary::from_cached(
        link.child_session.clone(),
        PathBuf::from(&link.child_session),
        link.project.clone(),
        "Worker".into(),
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
    session.harness = link.child_backend.clone();
    store.replace_sessions(&[session])?;
    drop(store);
    let store = StateStore::open_at(&database)?;
    let cached = store.cached_sessions("")?;
    assert_eq!(
        cached[0].model,
        Some(("opencode-go".into(), "glm-5.3-flash".into()))
    );
    assert_eq!(cached[0].thinking_level, None);

    let mut legacy = serde_json::to_value(&link).map_err(|error| error.to_string())?;
    legacy.as_object_mut().unwrap().remove("execution");
    let legacy: crate::agents::WorkerFamilyLink =
        serde_json::from_value(legacy).map_err(|error| error.to_string())?;
    assert!(legacy.execution.is_none());
    Ok(())
}
