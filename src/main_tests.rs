use super::*;

#[test]
fn reported_errors_return_failure_even_when_stderr_write_succeeds() {
    assert_eq!(
        fail_to(Vec::new(), "failed"),
        std::process::ExitCode::from(1)
    );
}

#[test]
fn startup_worker_command_loads_saved_app_proxy() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = app::persistence::StateStore::open_at(&directory.path().join("state.sqlite"))?;
    let proxy = "http://127.0.0.1:8118";
    store.save_network_proxy(Some(proxy))?;

    let command = startup_worker_command(directory.path(), Some(&store));

    assert_eq!(command.app_proxy.as_deref(), Some(proxy));
    assert_eq!(
        command.session_locator_root.as_deref(),
        Some(directory.path().join("session-locators").as_path())
    );
    Ok(())
}

#[test]
fn startup_worker_command_removes_cleared_app_proxy() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = app::persistence::StateStore::open_at(&directory.path().join("state.sqlite"))?;
    store.save_network_proxy(Some("https://proxy.example:8443"))?;
    store.save_network_proxy(None)?;

    let command = startup_worker_command(directory.path(), Some(&store));

    assert_eq!(command.app_proxy, None);
    Ok(())
}
