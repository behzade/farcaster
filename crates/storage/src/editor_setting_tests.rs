use super::*;

#[test]
fn editor_choice_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.load_editor_choice()?, EditorChoice::Neovim);
    store.save_editor_choice(EditorChoice::VsCode)?;
    drop(store);
    assert_eq!(
        StateStore::open_at(&database)?.load_editor_choice()?,
        EditorChoice::VsCode
    );
    Ok(())
}
