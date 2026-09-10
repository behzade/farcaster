use super::*;

#[test]
fn transcript_font_size_survives_reopen_and_rejects_invalid_values() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&path)?;
    let default = f32::from(THEME.type_scale.reading);
    assert_eq!(store.load_transcript_font_size()?, default);
    for size in [10.0, 19.0, 32.0, default] {
        store.save_transcript_font_size(size)?;
        assert_eq!(
            StateStore::open_at(&path)?.load_transcript_font_size()?,
            size
        );
    }
    for size in [9.0, 33.0, f32::NAN, f32::INFINITY] {
        assert!(store.save_transcript_font_size(size).is_err());
        assert_eq!(store.load_transcript_font_size()?, default);
    }
    for value in ["invalid", "NaN", "inf", "9", "33"] {
        store
            .connection
            .execute(
                "UPDATE meta SET value=?1 WHERE key='transcript_font_size'",
                [value],
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(store.load_transcript_font_size()?, default);
    }
    Ok(())
}
