use std::io::Write as _;
#[test]
#[ignore = "queries the configured live Cursor account"]
fn live_cursor_configuration_catalog() -> Result<(), String> {
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let catalog = super::super::load_configuration_catalog(
        &crate::AgentLaunchConfig::default(),
        crate::Backend::Cursor,
        &project,
    )?;
    assert!(!catalog.models.is_empty(), "Cursor returned no models");
    assert!(catalog.models.iter().all(|model| {
        model.provider == crate::Backend::Cursor.as_str() && !model.id.is_empty()
    }));
    writeln!(
        std::io::stderr().lock(),
        "Cursor catalog loaded {} models",
        catalog.models.len()
    )
    .expect("write test diagnostics");
    Ok(())
}
