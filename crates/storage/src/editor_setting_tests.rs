use super::*;

#[test]
fn editor_choice_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.load_editor_choice()?, EditorChoice::Neovim);
    for choice in EditorChoice::ALL {
        store.save_editor_choice(choice)?;
        assert_eq!(store.load_editor_choice()?, choice);
    }
    store.save_editor_choice(EditorChoice::Helix)?;
    drop(store);
    assert_eq!(
        StateStore::open_at(&database)?.load_editor_choice()?,
        EditorChoice::Helix
    );
    Ok(())
}

#[test]
fn custom_command_survives_editor_switches_and_rejects_invalid_updates()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.load_editor_command()?, "");
    let command = r#""/editor path/micro" -p"#;
    store.save_editor_command(command)?;
    store.save_editor_choice(EditorChoice::Custom)?;
    assert!(store.save_editor_command("micro '").is_err());
    store.save_editor_choice(EditorChoice::VsCode)?;
    drop(store);
    let store = StateStore::open_at(&database)?;
    assert_eq!(store.load_editor_choice()?, EditorChoice::VsCode);
    assert_eq!(store.load_editor_command()?, command);
    store.save_editor_choice(EditorChoice::Custom)?;
    assert_eq!(store.load_editor_command()?, command);
    Ok(())
}
