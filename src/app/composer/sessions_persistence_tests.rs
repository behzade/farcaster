use super::*;

#[test]
fn bound_draft_aliases_preserve_write_and_delete_order() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let mut store = StateStore::open_at(&temp.path().join("gui.sqlite3"))?;
    let mut draft =
        crate::projects::DraftSession::new("draft".into(), 0, temp.path().to_path_buf(), 1);
    draft.session_path = Some(temp.path().join("session.jsonl"));
    store.allocate_app_session_id(&draft)?;
    let bound = session_target(draft.session_path.as_ref().unwrap());
    let save = |target: String, text: &str| {
        PersistenceCommand::Save(ComposerRecord {
            target,
            text: text.into(),
            ..ComposerRecord::default()
        })
    };
    let mut pending = vec![
        save("draft:draft".into(), "older"),
        save(bound.clone(), "newer"),
    ];
    flush(&store, &mut pending);
    assert!(pending.is_empty());
    assert_eq!(store.load_composer_sessions()?[0].text, "newer");
    pending.extend([
        save(bound, "obsolete"),
        PersistenceCommand::Delete("draft:draft".into()),
    ]);
    flush(&store, &mut pending);
    assert!(store.load_composer_sessions()?.is_empty());
    Ok(())
}
