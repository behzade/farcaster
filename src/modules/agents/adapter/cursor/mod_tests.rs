#[test]
#[ignore = "queries the configured live Cursor account"]
fn live_cursor_configuration_catalog() -> Result<(), String> {
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let catalog = super::super::load_configuration_catalog(
        &crate::agents::AgentLaunchConfig::default(),
        super::PROFILE.backend,
        &project,
    )?;
    assert!(!catalog.models.is_empty(), "Cursor returned no models");
    assert!(
        catalog
            .models
            .iter()
            .all(|model| { model.provider == super::PROFILE.backend && !model.id.is_empty() })
    );
    eprintln!("Cursor catalog loaded {} models", catalog.models.len());
    Ok(())
}
