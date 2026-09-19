use super::*;

#[test]
fn command_sets_app_proxy_without_a_captured_environment() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let proxy = "http://127.0.0.1:8118";
    let command = AgentLaunchConfig {
        program: "/usr/bin/true".into(),
        app_proxy: Some(proxy.into()),
        ..AgentLaunchConfig::default()
    }
    .command(project.path())?;
    let environment = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(ToOwned::to_owned)))
        .collect::<Vec<_>>();

    for name in ["http_proxy", "https_proxy"] {
        assert!(environment.iter().any(|(actual, value)| {
            actual == name && value.as_deref() == Some(std::ffi::OsStr::new(proxy))
        }));
    }
    Ok(())
}
