use super::*;

#[test]
fn voice_defaults_on_and_preserves_an_explicit_choice() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    assert!(store.load_voice_enabled()?);
    store.save_voice_enabled(false)?;
    drop(store);
    let store = StateStore::open_at(&database)?;
    assert!(!store.load_voice_enabled()?);
    store.save_voice_enabled(true)?;
    drop(store);
    assert!(StateStore::open_at(&database)?.load_voice_enabled()?);
    Ok(())
}
