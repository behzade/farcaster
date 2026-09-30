use super::*;

#[test]
fn unsent_session_selections_survive_reopen_and_keep_sessions_separate() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let model = SessionControlSelection::Model(
        serde_json::from_value(serde_json::json!({
            "id":"chosen", "name":"Chosen", "provider":"openai"
        }))
        .map_err(|error| error.to_string())?,
    );
    let mut store = StateStore::open_at(&database)?;
    for selection in [
        model.clone(),
        SessionControlSelection::Effort(Some("high".into())),
        SessionControlSelection::ServiceTier(Some("fast".into())),
        SessionControlSelection::Effort(None),
        SessionControlSelection::ServiceTier(None),
    ] {
        store.save_session_control_selection(Backend::OpenCode, temp.path(), &first, &selection)?;
    }
    store.save_session_control_selection(
        Backend::OpenCode,
        temp.path(),
        &second,
        &SessionControlSelection::Effort(Some("low".into())),
    )?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    assert_eq!(
        store.load_session_control_selections(Backend::OpenCode, &first)?,
        [
            model,
            SessionControlSelection::Effort(None),
            SessionControlSelection::ServiceTier(None)
        ]
    );
    assert_eq!(
        store.load_session_control_selections(Backend::OpenCode, &second)?,
        [SessionControlSelection::Effort(Some("low".into()))]
    );
    assert!(
        store
            .load_session_control_selections(Backend::Pi, &first)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn session_merge_keeps_the_latest_selection_by_time() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for (path, level) in [(&first, "high"), (&second, "low")] {
        store.save_session_control_selection(
            Backend::Pi,
            temp.path(),
            path,
            &SessionControlSelection::Effort(Some(level.into())),
        )?;
    }
    store.with_connection(|connection| -> Result<(), String> {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let id = |path: &Path| -> Result<i64, String> {
            let path = sessions::normalize_session_path(path);
            tx.query_row(
                "SELECT id FROM sessions WHERE locator=?1",
                [path.to_string_lossy()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
        };
        let keep = id(&first)?;
        let other = id(&second)?;
        tx.execute(
            "UPDATE session_events SET t=CASE session_id WHEN ?1 THEN 200 ELSE 100 END",
            [keep],
        )
        .map_err(|error| error.to_string())?;
        super::super::identity::merge_session(&tx, keep, other)?;
        tx.commit().map_err(|error| error.to_string())
    })?;
    assert_eq!(
        store.load_session_control_selections(Backend::Pi, &first)?,
        [SessionControlSelection::Effort(Some("high".into()))]
    );
    Ok(())
}
